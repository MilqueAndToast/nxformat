#include "nx_reader.hpp"
#include "real_zstd.hpp"

#include <algorithm>
#include <array>
#include <atomic>
#include <chrono>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <limits>
#include <new>
#include <stdexcept>
#include <string>
#include <vector>

#if defined(_WIN32)
#include <windows.h>
#include <psapi.h>
#include <malloc.h>
#pragma comment(lib, "Psapi.lib")
#include <bcrypt.h>
#pragma comment(lib, "Bcrypt.lib")
#elif defined(__APPLE__)
#include <CommonCrypto/CommonDigest.h>
#include <sys/resource.h>
#else
#include <sys/resource.h>
#include <dlfcn.h>
#endif

static std::atomic<bool> count_allocations{false};
static std::atomic<std::uint64_t> allocation_count{0};

void* operator new(std::size_t size) {
    if (!size) size = 1;
    void* pointer = std::malloc(size);
    if (!pointer) throw std::bad_alloc();
    if (count_allocations.load(std::memory_order_relaxed)) allocation_count.fetch_add(1, std::memory_order_relaxed);
    return pointer;
}
void* operator new[](std::size_t size) { return ::operator new(size); }
void operator delete(void* pointer) noexcept { std::free(pointer); }
void operator delete[](void* pointer) noexcept { std::free(pointer); }
void operator delete(void* pointer, std::size_t) noexcept { std::free(pointer); }
void operator delete[](void* pointer, std::size_t) noexcept { std::free(pointer); }
#if defined(_WIN32)
void* operator new(std::size_t size, std::align_val_t alignment) {
    if (!size) size = 1;
    void* pointer = _aligned_malloc(size, static_cast<std::size_t>(alignment));
    if (!pointer) throw std::bad_alloc();
    if (count_allocations.load(std::memory_order_relaxed)) allocation_count.fetch_add(1, std::memory_order_relaxed);
    return pointer;
}
void operator delete(void* pointer, std::align_val_t) noexcept { _aligned_free(pointer); }
void operator delete(void* pointer, std::size_t, std::align_val_t) noexcept { _aligned_free(pointer); }
#else
void* operator new(std::size_t size, std::align_val_t alignment) {
    if (!size) size = 1;
    const auto align = static_cast<std::size_t>(alignment);
    const auto rounded = (size + align - 1) / align * align;
    void* pointer = std::aligned_alloc(align, rounded);
    if (!pointer) throw std::bad_alloc();
    if (count_allocations.load(std::memory_order_relaxed)) allocation_count.fetch_add(1, std::memory_order_relaxed);
    return pointer;
}
void operator delete(void* pointer, std::align_val_t) noexcept { std::free(pointer); }
void operator delete(void* pointer, std::size_t, std::align_val_t) noexcept { std::free(pointer); }
#endif

class FileSource final : public nx::RandomAccessSource {
public:
    explicit FileSource(const std::string& path) : file_(path, std::ios::binary) {
        file_.seekg(0, std::ios::end);
        const auto end = file_.tellg();
        size_ = end < 0 ? 0 : static_cast<std::uint64_t>(end);
        file_.seekg(0, std::ios::beg);
    }
    bool good() const { return file_.is_open(); }
    std::uint64_t size() const noexcept override { return size_; }
    bool set_data_region_from_header() {
        std::array<std::uint8_t, 16> fields{};
        if (!read_at(56, fields.data(), fields.size())) return false;
        data_begin_ = read_u64(fields.data());
        const auto data_size = read_u64(fields.data() + 8);
        if (data_begin_ > size_ || data_size > size_ - data_begin_) return false;
        data_end_ = data_begin_ + data_size;
        data_bytes_ = 0;
        return true;
    }
    void reset_data_bytes() { data_bytes_ = 0; }
    std::uint64_t data_bytes() const { return data_bytes_; }
    bool read_at(std::uint64_t offset, std::uint8_t* destination, std::size_t length) override {
        if (offset > size_ || length > size_ - offset || offset > static_cast<std::uint64_t>(std::numeric_limits<std::streamoff>::max())) return false;
        file_.clear();
        file_.seekg(static_cast<std::streamoff>(offset), std::ios::beg);
        if (!file_) return false;
        file_.read(reinterpret_cast<char*>(destination), static_cast<std::streamsize>(length));
        const bool ok = file_.gcount() == static_cast<std::streamsize>(length);
        if (ok) {
            const auto end = offset + length;
            const auto begin = std::max(offset, data_begin_);
            const auto overlap_end = std::min(end, data_end_);
            if (overlap_end > begin) data_bytes_ += overlap_end - begin;
        }
        return ok;
    }
private:
    static std::uint64_t read_u64(const std::uint8_t* p) {
        std::uint64_t value = 0;
        for (unsigned i = 0; i < 8; ++i) value |= std::uint64_t(p[i]) << (8 * i);
        return value;
    }
    std::ifstream file_;
    std::uint64_t size_ = 0;
    std::uint64_t data_begin_ = 0, data_end_ = 0, data_bytes_ = 0;
};

struct PhaseMetrics { double decode_us = 0, sha_us = 0; };
struct Sample {
    double microseconds = 0;
    std::uint64_t allocations = 0, offset = 0, data_bytes = 0;
    double decode_us = 0, sha_us = 0;
};

class NativeSha256 {
public:
#if defined(_WIN32)
    NativeSha256() {
        if (BCryptOpenAlgorithmProvider(&algorithm_, BCRYPT_SHA256_ALGORITHM, nullptr, 0) < 0) return;
        DWORD object_size = 0, received = 0;
        if (BCryptGetProperty(algorithm_, BCRYPT_OBJECT_LENGTH, reinterpret_cast<PUCHAR>(&object_size),
                              sizeof(object_size), &received, 0) < 0 || received != sizeof(object_size)) return;
        object_.resize(object_size);
        if (BCryptCreateHash(algorithm_, &hash_, object_.data(), object_size, nullptr, 0,
                             BCRYPT_HASH_REUSABLE_FLAG) >= 0) ready_ = true;
    }
    ~NativeSha256() {
        if (hash_) BCryptDestroyHash(hash_);
        if (algorithm_) BCryptCloseAlgorithmProvider(algorithm_, 0);
    }
    bool ready() const { return ready_; }
    const char* name() const { return "Windows CNG SHA-256"; }
    bool digest(const std::uint8_t* input, std::size_t size, std::uint8_t output[32]) {
        if (!ready_ || size > std::numeric_limits<ULONG>::max()) return false;
        return BCryptHashData(hash_, const_cast<PUCHAR>(input), static_cast<ULONG>(size), 0) >= 0 &&
               BCryptFinishHash(hash_, output, 32, 0) >= 0;
    }
private:
    BCRYPT_ALG_HANDLE algorithm_ = nullptr;
    BCRYPT_HASH_HANDLE hash_ = nullptr;
    std::vector<std::uint8_t> object_;
    bool ready_ = false;
#elif defined(__APPLE__)
    bool ready() const { return true; }
    const char* name() const { return "Apple CommonCrypto SHA-256"; }
    bool digest(const std::uint8_t* input, std::size_t size, std::uint8_t output[32]) {
        if (size > std::numeric_limits<CC_LONG>::max()) return false;
        return CC_SHA256(input, static_cast<CC_LONG>(size), output) != nullptr;
    }
#else
    struct MdContext;
    struct Md;
    using DigestFn = int (*)(const void*, std::size_t, unsigned char*, unsigned int*, const Md*, void*);
    using Sha256Fn = const Md* (*)();
    using VersionFn = const char* (*)(int);
    NativeSha256() { load(); }
    ~NativeSha256() { if (library_) dlclose(library_); }
    bool ready() const { return library_ && digest_ && sha256_; }
    const char* name() const { return "OpenSSL EVP SHA-256"; }
    bool digest(const std::uint8_t* input, std::size_t size, std::uint8_t output[32]) {
        unsigned int written = 0;
        return ready() && digest_(input, size, output, &written, sha256_(), nullptr) == 1 && written == 32;
    }
    const char* version() const { return version_ ? version_(0) : "OpenSSL (unknown version)"; }
private:
    void* library_ = nullptr;
    DigestFn digest_ = nullptr;
    Sha256Fn sha256_ = nullptr;
    VersionFn version_ = nullptr;
    template<class Function> static Function function_pointer(void* address) {
        Function result = nullptr;
        static_assert(sizeof(result) == sizeof(address), "unexpected function pointer size");
        std::memcpy(&result, &address, sizeof(result));
        return result;
    }
    void load() {
        const char* candidates[] = {"libcrypto.so.3", "libcrypto.so"};
        for (const char* candidate : candidates) if ((library_ = dlopen(candidate, RTLD_NOW | RTLD_LOCAL))) break;
        if (!library_) return;
        digest_ = function_pointer<DigestFn>(dlsym(library_, "EVP_Digest"));
        sha256_ = function_pointer<Sha256Fn>(dlsym(library_, "EVP_sha256"));
        version_ = function_pointer<VersionFn>(dlsym(library_, "OpenSSL_version"));
    }
#endif
};

struct ReaderCallbackContext {
    RealZstd* decoder = nullptr;
    NativeSha256* native_sha = nullptr;
    PhaseMetrics* metrics = nullptr;
    const char* sha_provider = "none";
};

static FileSource* active_source = nullptr;
static PhaseMetrics active_phase_metrics{};

static bool timed_zstd_decode(void* opaque, const std::uint8_t* frame, std::size_t frame_size,
                              std::uint8_t* output, std::size_t capacity,
                              std::size_t* written, std::size_t* consumed) {
    auto& context = *static_cast<ReaderCallbackContext*>(opaque);
    const auto start = std::chrono::steady_clock::now();
    const bool ok = RealZstd::decode(context.decoder, frame, frame_size, output, capacity, written, consumed);
    const auto finish = std::chrono::steady_clock::now();
    if (context.metrics) context.metrics->decode_us += std::chrono::duration<double, std::micro>(finish - start).count();
    return ok;
}

static bool timed_sha256(void* opaque, const std::uint8_t* input, std::size_t size, std::uint8_t output[32]) {
    auto& context = *static_cast<ReaderCallbackContext*>(opaque);
    const auto start = std::chrono::steady_clock::now();
    bool ok = false;
    if (std::string(context.sha_provider) == "portable") {
        const auto digest = nx::Sha256::digest(input, size);
        std::copy(digest.begin(), digest.end(), output);
        ok = true;
    } else if (context.native_sha) {
        ok = context.native_sha->digest(input, size, output);
    }
    const auto finish = std::chrono::steady_clock::now();
    if (context.metrics) context.metrics->sha_us += std::chrono::duration<double, std::micro>(finish - start).count();
    return ok;
}

static std::uint64_t peak_memory_bytes() {
#if defined(_WIN32)
    PROCESS_MEMORY_COUNTERS_EX counters{};
    counters.cb = sizeof(counters);
    if (!GetProcessMemoryInfo(GetCurrentProcess(), reinterpret_cast<PROCESS_MEMORY_COUNTERS*>(&counters), sizeof(counters))) return 0;
    return static_cast<std::uint64_t>(counters.PeakWorkingSetSize);
#else
    struct rusage usage{};
    if (getrusage(RUSAGE_SELF, &usage) != 0) return 0;
#if defined(__APPLE__)
    return static_cast<std::uint64_t>(usage.ru_maxrss);
#else
    return static_cast<std::uint64_t>(usage.ru_maxrss) * 1024;
#endif
#endif
}

template<class Function>
static Sample timed(Function&& function) {
    allocation_count.store(0, std::memory_order_relaxed);
    count_allocations.store(true, std::memory_order_relaxed);
    active_phase_metrics = {};
    if (active_source) active_source->reset_data_bytes();
    const auto start = std::chrono::steady_clock::now();
    const bool ok = function();
    const auto finish = std::chrono::steady_clock::now();
    count_allocations.store(false, std::memory_order_relaxed);
    if (!ok) throw std::runtime_error("benchmark operation returned an error");
    return {std::chrono::duration<double, std::micro>(finish - start).count(),
            allocation_count.load(std::memory_order_relaxed), 0,
            active_source ? active_source->data_bytes() : 0,
            active_phase_metrics.decode_us, active_phase_metrics.sha_us};
}

static double percentile(std::vector<double> values, double p) {
    std::sort(values.begin(), values.end());
    const auto rank = static_cast<std::size_t>(std::ceil(p * static_cast<double>(values.size())));
    return values[std::max<std::size_t>(1, rank) - 1];
}

template<class Projection>
static double percentile_sample(const std::vector<Sample>& samples, double p, Projection projection) {
    std::vector<double> values;
    values.reserve(samples.size());
    for (const auto& sample : samples) values.push_back(projection(sample));
    return percentile(std::move(values), p);
}

static void report(const std::string& profile, const std::string& codec, unsigned block_kib,
                   bool verify, const std::string& sha_provider,
                   const std::string& path_mode, const std::string& operation,
                   const std::vector<Sample>& samples, std::uint64_t bytes_per_sample,
                   std::uint64_t archive_bytes) {
    std::vector<double> times;
    double total_seconds = 0;
    std::uint64_t total_allocations = 0, total_data_bytes = 0;
    times.reserve(samples.size());
    for (const auto& sample : samples) {
        times.push_back(sample.microseconds);
        total_seconds += sample.microseconds / 1'000'000.0;
        total_allocations += sample.allocations;
        total_data_bytes += sample.data_bytes;
    }
    const double throughput = total_seconds > 0 ? (static_cast<double>(bytes_per_sample) * samples.size() / total_seconds / (1024.0 * 1024.0)) : 0;
    std::cout << profile << ',' << codec << ',' << block_kib << ',' << (verify ? 1 : 0) << ',' << sha_provider << ',' << path_mode << ',' << operation << ','
              << samples.size() << ',' << std::fixed << std::setprecision(3)
              << percentile(times, 0.50) << ',' << percentile(times, 0.95) << ',' << percentile(times, 0.99) << ','
              << throughput << ',' << (static_cast<double>(total_allocations) / samples.size()) << ','
              << peak_memory_bytes() << ',' << archive_bytes << ",warm_file_cache,"
              << (static_cast<double>(total_data_bytes) / samples.size()) << ','
              << percentile_sample(samples, 0.50, [](const Sample& sample) { return sample.decode_us; }) << ','
              << percentile_sample(samples, 0.50, [](const Sample& sample) { return sample.sha_us; }) << '\n';
    for (std::size_t i = 0; i < samples.size(); ++i) {
        const auto& sample = samples[i];
        std::cout << "SAMPLE," << profile << ',' << codec << ',' << block_kib << ',' << (verify ? 1 : 0) << ','
                  << sha_provider << ',' << path_mode << ',' << operation << ',' << i << ',' << sample.offset << ',' << bytes_per_sample << ','
                  << std::fixed << std::setprecision(3) << sample.microseconds << ',' << sample.allocations << ','
                  << sample.data_bytes << ',' << sample.decode_us << ',' << sample.sha_us << '\n';
    }
}

static std::uint64_t next_random(std::uint64_t& state) {
    state ^= state << 13; state ^= state >> 7; state ^= state << 17; return state;
}

static bool read_whole_file(const std::string& path) {
    std::ifstream file(path, std::ios::binary);
    std::array<char, 64 * 1024> buffer{};
    while (file) file.read(buffer.data(), static_cast<std::streamsize>(buffer.size()));
    return file.eof();
}

static int run_cache_study(FileSource& source, RealZstd& decoder, NativeSha256& native_sha,
                           const std::string& profile, const std::string& codec,
                           unsigned block_kib, std::uint64_t archive_bytes) {
    ReaderCallbackContext callback_context{&decoder, &native_sha, &active_phase_metrics, "none"};
    nx::OpenOptions base_options;
    base_options.zstd = &timed_zstd_decode;
    base_options.zstd_context = &callback_context;
    std::vector<std::string> providers{"crc32c"};
    constexpr std::uint64_t logical_bytes = 16ull * 1024 * 1024;

    for (const auto& provider : providers) {
        callback_context.sha_provider = provider.c_str();
        for (const std::size_t read_size : {std::size_t{4096}, std::size_t{64 * 1024}}) {
            constexpr unsigned sample_count = 120;
            std::vector<std::uint64_t> offsets(sample_count);
            std::uint64_t state = 0x4e58434143484531ull + read_size + block_kib;
            for (auto& offset : offsets) offset = next_random(state) % (logical_bytes - read_size + 1);
            std::vector<std::uint8_t> output(read_size);

            nx::OpenOptions uncached_options = base_options;
            uncached_options.verify_read_blocks = true;
            uncached_options.block_sha256 = &timed_sha256;
            uncached_options.block_sha256_context = &callback_context;
            auto uncached_open = nx::Archive::open(source, uncached_options);
            if (!uncached_open || !uncached_open.value->mount()) return 2;
            nx::Archive uncached = std::move(*uncached_open.value);
            const auto uncached_file = uncached.resolve_file(0, "payload.bin");
            if (!uncached_file) return 2;
            std::vector<Sample> samples;
            samples.reserve(sample_count);
            for (const auto offset : offsets) {
                auto sample = timed([&]() {
                    return uncached.read_file(uncached_file.value->id, offset, read_size,
                                              output.data(), output.size()).ok();
                });
                sample.offset = offset;
                samples.push_back(sample);
            }
            report(profile, codec, block_kib, true, provider, "file_id",
                   "cache_disabled_random_" + std::to_string(read_size / 1024) + "k",
                   samples, read_size, archive_bytes);

            nx::OpenOptions cached_options = uncached_options;
            cached_options.max_cached_block_bytes = 8 * 1024 * 1024;
            cached_options.max_cached_blocks = 128;
            samples.clear();
            for (const auto offset : offsets) {
                auto opened = nx::Archive::open(source, cached_options);
                if (!opened || !opened.value->mount()) return 2;
                nx::Archive archive = std::move(*opened.value);
                const auto file = archive.resolve_file(0, "payload.bin");
                if (!file) return 2;
                auto sample = timed([&]() {
                    return archive.read_file(file.value->id, offset, read_size,
                                            output.data(), output.size()).ok();
                });
                sample.offset = offset;
                samples.push_back(sample);
            }
            report(profile, codec, block_kib, true, provider, "file_id",
                   "cache_enabled_cold_miss_random_" + std::to_string(read_size / 1024) + "k",
                   samples, read_size, archive_bytes);

            auto warm_open = nx::Archive::open(source, cached_options);
            if (!warm_open || !warm_open.value->mount()) return 2;
            nx::Archive warm = std::move(*warm_open.value);
            const auto warm_file = warm.resolve_file(0, "payload.bin");
            if (!warm_file) return 2;
            samples.clear();
            for (const auto offset : offsets) {
                if (!warm.read_file(warm_file.value->id, offset, read_size,
                                    output.data(), output.size())) return 2;
                auto sample = timed([&]() {
                    return warm.read_file(warm_file.value->id, offset, read_size,
                                          output.data(), output.size()).ok();
                });
                sample.offset = offset;
                samples.push_back(sample);
            }
            report(profile, codec, block_kib, true, provider, "file_id",
                   "cache_enabled_warm_hit_random_" + std::to_string(read_size / 1024) + "k",
                   samples, read_size, archive_bytes);
        }
    }
    return 0;
}

int main(int argc, char** argv) {
    if (argc != 5 && !(argc == 6 && std::string(argv[5]) == "cache-study")) {
        std::cerr << "usage: nx_reader_benchmark <archive.nx> <profile> <raw|zstd> <block-kib>\n";
        return 2;
    }
    const std::string path = argv[1], profile = argv[2], codec = argv[3];
    const unsigned block_kib = static_cast<unsigned>(std::strtoul(argv[4], nullptr, 10));
    FileSource source(path);
    if (!source.good() || !source.set_data_region_from_header() || !read_whole_file(path)) return 2;
    active_source = &source;
    RealZstd decoder;
    if (!decoder.ready()) { std::cerr << decoder.error() << '\n'; return 2; }
    std::cerr << "zstd_library_version=" << decoder.version() << '\n';
    NativeSha256 native_sha;
    if (argc == 6) {
        std::cout << "profile,codec,block_kib,verify_read_sha256,sha_provider,read_mode,operation,samples,p50_us,p95_us,p99_us,throughput_mib_s,allocations_per_sample,peak_working_set_bytes,archive_bytes,cache_state,data_region_bytes_per_sample,decode_p50_us,sha_p50_us\n";
        return run_cache_study(source, decoder, native_sha, profile, codec, block_kib, source.size());
    }
    const std::array<std::uint8_t, 3> abc = {'a','b','c'};
    const std::array<std::uint8_t, 32> expected_abc = {0xba,0x78,0x16,0xbf,0x8f,0x01,0xcf,0xea,
        0x41,0x41,0x40,0xde,0x5d,0xae,0x22,0x23,0xb0,0x03,0x61,0xa3,0x96,0x17,0x7a,0x9c,
        0xb4,0x10,0xff,0x61,0xf2,0x00,0x15,0xad};
    std::array<std::uint8_t, 32> native_vector{};
    if (native_sha.ready()) {
        if (!native_sha.digest(abc.data(), abc.size(), native_vector.data()) || native_vector != expected_abc ||
            native_vector != nx::Sha256::digest(abc.data(), abc.size())) {
            std::cerr << "native SHA-256 provider failed independent abc vector\n"; return 2;
        }
        std::cerr << "native_sha_provider=" << native_sha.name() << "\n";
    } else {
        std::cerr << "native_sha_provider=unavailable; native read cases skipped\n";
    }
    ReaderCallbackContext callback_context{&decoder, &native_sha, &active_phase_metrics, "none"};
    nx::OpenOptions base_options;
    base_options.zstd = &timed_zstd_decode;
    base_options.zstd_context = &callback_context;
    const std::uint64_t archive_bytes = source.size();
    constexpr std::uint64_t logical_bytes = 16ull * 1024 * 1024;

    std::vector<Sample> samples;
    for (unsigned i = 0; i < 60; ++i) {
        samples.push_back(timed([&]() {
            auto opened = nx::Archive::open(source, base_options);
            if (!opened) return false;
            const auto& archive = *opened.value;
            return archive.title(0) == "NX synthetic benchmark" && archive.localized_title(0) == "NX benchmark";
        }));
    }
    report(profile, codec, block_kib, false, "none", "n/a", "open_catalog_titles", samples, 0, archive_bytes);

    samples.clear();
    for (unsigned i = 0; i < 40; ++i) {
        auto opened = nx::Archive::open(source, base_options);
        if (!opened) return 2;
        nx::Archive archive = std::move(*opened.value);
        samples.push_back(timed([&]() {
            auto media = archive.read_media(0);
            return media && media.value->size() > 8 && (*media.value)[0] == 0x89;
        }));
    }
    report(profile, codec, block_kib, false, "none", "n/a", "read_icon", samples, 73, archive_bytes);

    samples.clear();
    for (unsigned i = 0; i < 30; ++i) {
        auto opened = nx::Archive::open(source, base_options);
        if (!opened) return 2;
        nx::Archive archive = std::move(*opened.value);
        samples.push_back(timed([&]() { return archive.mount().ok(); }));
    }
    report(profile, codec, block_kib, false, "none", "n/a", "mount", samples, 0, archive_bytes);

    std::vector<std::string> providers = {"none", "crc32c"};
    for (const std::string& sha_provider : providers) {
        const bool verify = sha_provider != "none";
        callback_context.sha_provider = sha_provider.c_str();
        nx::OpenOptions options = base_options;
        options.verify_read_blocks = verify;
        if (verify) {
            options.block_sha256 = &timed_sha256;
            options.block_sha256_context = &callback_context;
        }
        auto opened = nx::Archive::open(source, options);
        if (!opened) { std::cerr << "read benchmark open failed: " << opened.error.message << '\n'; return 2; }
        nx::Archive archive = std::move(*opened.value);
        const auto mounted = archive.mount();
        if (!mounted) { std::cerr << "read benchmark mount failed: " << mounted.error.message << '\n'; return 2; }
        const auto resolved = archive.resolve_file(0, "payload.bin");
        if (!resolved || resolved.value->size != logical_bytes) {
            std::cerr << "read benchmark resolve failed: " << (resolved ? "unexpected file size" : resolved.error.message)
                      << "; actual=" << (resolved ? resolved.value->size : 0) << ", expected=" << logical_bytes << '\n';
            return 2;
        }
        for (const std::size_t read_size : {std::size_t{4096}, std::size_t{64 * 1024}, std::size_t{1024 * 1024}}) {
            const unsigned count = read_size == 4096 ? 200 : (read_size == 64 * 1024 ? 140 : 100);
            std::vector<std::uint64_t> offsets(count);
            std::uint64_t state = 0x4e58524541444552ull + read_size + block_kib;
            for (auto& offset : offsets) offset = next_random(state) % (logical_bytes - read_size + 1);
            std::vector<std::uint8_t> output(read_size);
            for (const std::string mode : {"path", "file_id"}) {
                samples.clear();
                for (const auto offset : offsets) {
                    auto sample = timed([&]() {
                        const auto result = mode == "path"
                            ? archive.read_file(0, "payload.bin", offset, read_size, output.data(), output.size())
                            : archive.read_file(resolved.value->id, offset, read_size, output.data(), output.size());
                        return result.ok();
                    });
                    sample.offset = offset;
                    samples.push_back(sample);
                }
                report(profile, codec, block_kib, verify, sha_provider, mode, "random_" + std::to_string(read_size / 1024) + "k", samples, read_size, archive_bytes);
            }
        }
        std::vector<std::uint8_t> output(1024 * 1024);
        for (const std::string mode : {"path", "file_id"}) {
            samples.clear();
            for (unsigned pass = 0; pass < 12; ++pass) {
                for (std::uint64_t offset = 0; offset < logical_bytes; offset += output.size()) {
                    auto sample = timed([&]() {
                        const auto result = mode == "path"
                            ? archive.read_file(0, "payload.bin", offset, output.size(), output.data(), output.size())
                            : archive.read_file(resolved.value->id, offset, output.size(), output.data(), output.size());
                        return result.ok();
                    });
                    sample.offset = offset;
                    samples.push_back(sample);
                }
            }
            report(profile, codec, block_kib, verify, sha_provider, mode, "sequential_1m_chunks", samples, output.size(), archive_bytes);
        }
    }
    return 0;
}
