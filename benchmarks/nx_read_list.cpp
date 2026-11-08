// Times NX reads listed in a file, with the shipped C++ reader.
// Usage: nx_read_list ARCHIVE.nx SNAPSHOT READS.tsv OUT.tsv [CACHE_MIB]
// CACHE_MIB turns on the reader's verified-block cache with that byte budget.
// READS.tsv: path<TAB>offset<TAB>length per line. OUT.tsv adds the time in ns.
#include "nx_reader.hpp"
#include "real_zstd.hpp"

#include <chrono>
#include <cstdio>
#include <fstream>
#include <iostream>
#include <sstream>
#include <string>
#include <vector>

class FileSource final : public nx::RandomAccessSource {
public:
    explicit FileSource(const char* path) {
        file_ = std::fopen(path, "rb");
        if (file_) {
            _fseeki64(file_, 0, SEEK_END);
            size_ = static_cast<std::uint64_t>(_ftelli64(file_));
        }
    }
    ~FileSource() override { if (file_) std::fclose(file_); }
    bool good() const { return file_ != nullptr; }
    std::uint64_t size() const noexcept override { return size_; }
    bool read_at(std::uint64_t offset, std::uint8_t* destination, std::size_t length) override {
        if (_fseeki64(file_, static_cast<long long>(offset), SEEK_SET) != 0) return false;
        return std::fread(destination, 1, length, file_) == length;
    }
private:
    std::FILE* file_ = nullptr;
    std::uint64_t size_ = 0;
};

int main(int argc, char** argv) {
    if (argc != 5 && argc != 6) {
        std::cerr << "usage: nx_read_list ARCHIVE.nx SNAPSHOT READS.tsv OUT.tsv [CACHE_MIB]\n";
        return 2;
    }
    FileSource source(argv[1]);
    if (!source.good()) { std::cerr << "cannot open archive\n"; return 2; }
    RealZstd zstd;
    nx::OpenOptions options;
    options.zstd = &RealZstd::decode;
    options.zstd_context = &zstd;
    if (argc == 6) {
        // The reader's verified-block cache, sized in MiB (64 KiB blocks at least).
        options.max_cached_block_bytes = std::stoull(argv[5]) << 20;
        options.max_cached_blocks = static_cast<std::size_t>(options.max_cached_block_bytes / 65536 + 1);
    }
    const auto open_started = std::chrono::steady_clock::now();
    auto opened = nx::Archive::open(source, options);
    if (!opened) { std::cerr << "open: " << opened.error.message << '\n'; return 2; }
    nx::Archive archive = std::move(*opened.value);
    const auto mounted = archive.mount();
    if (!mounted) { std::cerr << "mount: " << mounted.error.message << '\n'; return 2; }
    const auto open_ns = std::chrono::duration_cast<std::chrono::nanoseconds>(
        std::chrono::steady_clock::now() - open_started).count();
    std::cout << "open_mount_ns\t" << open_ns << '\n';
    const auto snapshot = static_cast<std::uint32_t>(std::stoul(argv[2]));
    std::ifstream in(argv[3]);
    std::ofstream out(argv[4]);
    std::string line;
    std::vector<std::uint8_t> buffer;
    unsigned failed = 0;
    while (std::getline(in, line)) {
        std::istringstream fields(line);
        std::string path;
        std::uint64_t offset = 0, length = 0;
        std::getline(fields, path, '\t');
        fields >> offset >> length;
        buffer.resize(length);
        const auto started = std::chrono::steady_clock::now();
        const auto result = archive.read_file(snapshot, path, offset, length, buffer.data(), buffer.size());
        const auto ns = std::chrono::duration_cast<std::chrono::nanoseconds>(
            std::chrono::steady_clock::now() - started).count();
        if (!result.ok()) {
            if (++failed <= 3) std::cerr << "read failed: " << path << ": " << result.error.message << '\n';
            out << path << '\t' << offset << '\t' << length << "\t-1\n";
            continue;
        }
        out << path << '\t' << offset << '\t' << length << '\t' << ns << '\n';
    }
    std::cout << "failed\t" << failed << '\n';
    return failed ? 1 : 0;
}
