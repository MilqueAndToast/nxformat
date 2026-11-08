#include "nx_reader.hpp"
#include "real_zstd.hpp"

#include <cstdint>
#include <fstream>
#include <iostream>
#include <limits>
#include <mutex>
#include <string>

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
    bool read_at(std::uint64_t offset, std::uint8_t* destination, std::size_t length) override {
        std::lock_guard<std::mutex> lock(mutex_);
        if (offset > size_ || length > size_ - offset ||
            offset > static_cast<std::uint64_t>(std::numeric_limits<std::streamoff>::max())) return false;
        file_.clear();
        file_.seekg(static_cast<std::streamoff>(offset), std::ios::beg);
        if (!file_) return false;
        file_.read(reinterpret_cast<char*>(destination), static_cast<std::streamsize>(length));
        return file_.gcount() == static_cast<std::streamsize>(length);
    }
private:
    std::ifstream file_;
    std::uint64_t size_ = 0;
    std::mutex mutex_;
};

int main(int argc, char** argv) {
    if (argc != 3) return 2;
    FileSource source(argv[1]);
    if (!source.good()) return 1;
    RealZstd zstd;
    if (!zstd.ready()) {
        std::cerr << "real Zstandard runtime is required\n";
        return 2;
    }
    nx::OpenOptions options;
    options.zstd = &RealZstd::decode;
    options.zstd_context = &zstd;
    const std::string selection = argv[2];
    auto opened = nx::Archive::open(source, options);
    if (selection == "reject") return opened ? 1 : 0;
    if (!opened) {
        std::cerr << "C++ reader open failed: " << opened.error.message << '\n';
        return 1;
    }
    nx::Archive archive = std::move(*opened.value);
    const bool update_metadata = selection == "update";
    const bool fallback = selection == "fallback";
    if ((selection != "base" && !update_metadata && !fallback) || archive.applications().size() != 1) return 1;
    if (archive.title(0) != (update_metadata ? "Synthetic update" : "Repack synthetic base")) return 1;
    if (!archive.mount()) return 1;
    const auto expected_snapshots = selection == "fallback" ? 2u : 3u;
    if (archive.snapshots().size() != expected_snapshots) return 1;
    const auto verified = archive.verify_all();
    if (!verified) {
        std::cerr << "C++ full verification failed: " << verified.error.message << '\n';
        return 1;
    }
    const auto icon = archive.read_media(archive.applications()[0].icon_index);
    const std::string expected_icon = update_metadata ? "synthetic update JPEG" : "synthetic PNG body";
    if (!icon || std::string(icon.value->begin(), icon.value->end()) != expected_icon) return 1;
    std::cout << "PASS C++17 open, metadata, mount, snapshots, media and full verification\n";
    return 0;
}
