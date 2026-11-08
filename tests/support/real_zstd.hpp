#ifndef NX_TEST_REAL_ZSTD_HPP_INCLUDED
#define NX_TEST_REAL_ZSTD_HPP_INCLUDED

#include <cstddef>
#include <cstdint>
#include <cstring>
#include <string>

#if defined(_WIN32)
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#else
#include <dlfcn.h>
#endif

class RealZstd {
public:
    using Create = void* (*)();
    using Free = std::size_t (*)(void*);
    using SetParameter = std::size_t (*)(void*, int, int);
    using Decompress = std::size_t (*)(void*, void*, std::size_t, const void*, std::size_t);
    using IsError = unsigned (*)(std::size_t);
    using FindFrameSize = std::size_t (*)(const void*, std::size_t);
    using FrameContentSize = unsigned long long (*)(const void*, std::size_t);
    using DictionaryId = unsigned (*)(const void*, std::size_t);
    using VersionString = const char* (*)();
    using ContextSize = std::size_t (*)(const void*);

    RealZstd() { load(); }
    ~RealZstd() {
        if (context_ && free_) (void)free_(context_);
#if defined(_WIN32)
        if (library_) FreeLibrary(static_cast<HMODULE>(library_));
#else
        if (library_) dlclose(library_);
#endif
    }
    RealZstd(const RealZstd&) = delete;
    RealZstd& operator=(const RealZstd&) = delete;

    bool ready() const noexcept { return context_ != nullptr; }
    const std::string& error() const noexcept { return error_; }
    const char* version() const noexcept { return version_string_ ? version_string_() : "unknown"; }
    std::size_t context_bytes() const noexcept { return context_size_ && context_ ? context_size_(context_) : 0; }

    static bool decode(void* opaque, const std::uint8_t* frame, std::size_t frame_size,
                       std::uint8_t* output, std::size_t capacity,
                       std::size_t* written, std::size_t* consumed) {
        auto* self = static_cast<RealZstd*>(opaque);
        if (!self || !self->ready() || !frame || !output || !written || !consumed || frame_size < 6) return false;
        // Standard Zstandard frame magic only; reject skippable frames.
        if (frame[0] != 0x28 || frame[1] != 0xb5 || frame[2] != 0x2f || frame[3] != 0xfd) return false;
        constexpr unsigned long long unknown_size = 0xffffffffffffffffull;
        constexpr unsigned long long error_size = 0xfffffffffffffffeull;
        const auto content_size = self->frame_content_size_(frame, frame_size);
        if (content_size == unknown_size || content_size == error_size || content_size != capacity) return false;
        if (self->dictionary_id_(frame, frame_size) != 0 || frame_size < 5) return false;
        const std::uint8_t descriptor = frame[4];
        if (descriptor & 0x18u) return false; // Reserved descriptor bits.
        const unsigned content_flag = descriptor >> 6;
        const bool single_segment = (descriptor & 0x20u) != 0;
        const unsigned dictionary_flag = descriptor & 0x03u;
        if (dictionary_flag != 0) return false;
        std::size_t cursor = 5;
        std::uint64_t window_size = content_size;
        if (!single_segment) {
            if (cursor >= frame_size) return false;
            const unsigned wd = frame[cursor++];
            const unsigned exponent = wd >> 3;
            const std::uint64_t base = std::uint64_t{1} << (10 + exponent);
            window_size = base + (base >> 3) * (wd & 7u);
        }
        const std::size_t fcs_size = content_flag == 0 ? (single_segment ? 1u : 0u) :
            (content_flag == 1 ? 2u : (content_flag == 2 ? 4u : 8u));
        if (fcs_size > frame_size - cursor) return false;
        if (window_size > (1ull << 20)) return false;
        const auto exact_size = self->find_frame_size_(frame, frame_size);
        if (self->is_error_(exact_size) || exact_size != frame_size) return false;
        const auto result = self->decompress_(self->context_, output, capacity, frame, frame_size);
        if (self->is_error_(result) || result != content_size) return false;
        *written = result;
        *consumed = frame_size;
        return true;
    }

private:
    void* library_ = nullptr;
    void* context_ = nullptr;
    Free free_ = nullptr;
    SetParameter set_parameter_ = nullptr;
    Decompress decompress_ = nullptr;
    IsError is_error_ = nullptr;
    FindFrameSize find_frame_size_ = nullptr;
    FrameContentSize frame_content_size_ = nullptr;
    DictionaryId dictionary_id_ = nullptr;
    VersionString version_string_ = nullptr;
    ContextSize context_size_ = nullptr;
    std::string error_;

    template<class Function>
    static Function function_pointer(void* symbol) {
        Function result = nullptr;
        static_assert(sizeof(result) == sizeof(symbol), "function/data pointer size mismatch on this test platform");
        std::memcpy(&result, &symbol, sizeof(result));
        return result;
    }

    void load() {
#if defined(_WIN32)
        char configured_path[4096]{};
        const DWORD configured_length = GetEnvironmentVariableA("NX_ZSTD_LIBRARY", configured_path, static_cast<DWORD>(sizeof(configured_path)));
        const char* configured = configured_length && configured_length < sizeof(configured_path) ? configured_path : nullptr;
        const char* candidates[] = {configured, "libzstd.dll", "zstd.dll", "C:\\Program Files\\Git\\mingw64\\bin\\libzstd.dll"};
        for (const char* candidate : candidates) {
            if (candidate && *candidate) {
                library_ = LoadLibraryA(candidate);
                if (library_) break;
            }
        }
        auto symbol = [this](const char* name) -> void* {
            return library_ ? reinterpret_cast<void*>(GetProcAddress(static_cast<HMODULE>(library_), name)) : nullptr;
        };
#else
#if defined(__APPLE__)
        // Homebrew libraries are not guaranteed to be in dyld's default path.
        const char* candidates[] = {
            "libzstd.dylib",
            "/opt/homebrew/opt/zstd/lib/libzstd.dylib",
            "/usr/local/opt/zstd/lib/libzstd.dylib",
            "/opt/homebrew/lib/libzstd.dylib",
            "/usr/local/lib/libzstd.dylib"
        };
#else
        const char* candidates[] = {"libzstd.so.1", "libzstd.so"};
#endif
        for (const char* candidate : candidates) {
            library_ = dlopen(candidate, RTLD_NOW | RTLD_LOCAL);
            if (library_) break;
        }
        auto symbol = [this](const char* name) -> void* { return library_ ? dlsym(library_, name) : nullptr; };
#endif
        if (!library_) { error_ = "could not load a system Zstandard library"; return; }
        auto create = function_pointer<Create>(symbol("ZSTD_createDCtx"));
        free_ = function_pointer<Free>(symbol("ZSTD_freeDCtx"));
        set_parameter_ = function_pointer<SetParameter>(symbol("ZSTD_DCtx_setParameter"));
        decompress_ = function_pointer<Decompress>(symbol("ZSTD_decompressDCtx"));
        is_error_ = function_pointer<IsError>(symbol("ZSTD_isError"));
        find_frame_size_ = function_pointer<FindFrameSize>(symbol("ZSTD_findFrameCompressedSize"));
        frame_content_size_ = function_pointer<FrameContentSize>(symbol("ZSTD_getFrameContentSize"));
        dictionary_id_ = function_pointer<DictionaryId>(symbol("ZSTD_getDictID_fromFrame"));
        version_string_ = function_pointer<VersionString>(symbol("ZSTD_versionString"));
        context_size_ = function_pointer<ContextSize>(symbol("ZSTD_sizeof_DCtx"));
        if (!create || !free_ || !set_parameter_ || !decompress_ || !is_error_ || !find_frame_size_ || !frame_content_size_ || !dictionary_id_ || !version_string_) {
            error_ = "system Zstandard library lacks required decoder entry points";
            return;
        }
        context_ = create();
        if (!context_ || is_error_(set_parameter_(context_, 100, 20)) != 0) {
            error_ = "could not create a Zstandard decoder limited to a 1 MiB window";
            if (context_) { (void)free_(context_); context_ = nullptr; }
        }
    }
};

#endif
