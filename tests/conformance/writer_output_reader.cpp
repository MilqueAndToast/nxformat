#include "nx_reader.hpp"
#include "real_zstd.hpp"

#include <algorithm>
#include <cstdint>
#include <fstream>
#include <iostream>
#include <limits>
#include <mutex>
#include <string>
#include <string_view>
#include <vector>

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

static bool check(bool condition, const char* message) {
    if (!condition) std::cerr << "FAIL " << message << '\n';
    return condition;
}

static std::uint8_t pattern(std::uint64_t offset) {
    return static_cast<std::uint8_t>((offset * 17u + 11u) & 0xffu);
}

static bool read_pattern(nx::Archive& archive, std::uint32_t snapshot, std::string_view path,
                         std::uint64_t file_size, std::uint64_t offset) {
    const auto info = archive.resolve_file(snapshot, path);
    if (!check(info && info.value->size == file_size, "writer output path/size resolution")) return false;
    const std::size_t length = static_cast<std::size_t>(std::min<std::uint64_t>(193, file_size - offset));
    std::vector<std::uint8_t> output(length, 0xa5);
    const auto by_path = archive.read_file(snapshot, path, offset, length, output.data(), output.size());
    if (!check(by_path.ok(), "writer output path random read")) return false;
    for (std::size_t i = 0; i < length; ++i)
        if (output[i] != pattern(offset + i)) return check(false, "writer output byte differs from expected stream");
    std::fill(output.begin(), output.end(), std::uint8_t{0xa5});
    const auto by_id = archive.read_file(info.value->id, offset, length, output.data(), output.size());
    if (!check(by_id.ok(), "writer output resolved FileId random read")) return false;
    for (std::size_t i = 0; i < length; ++i)
        if (output[i] != pattern(offset + i)) return check(false, "resolved FileId byte differs from expected stream");
    return true;
}

static bool read_text(nx::Archive& archive, std::uint32_t snapshot, std::string_view path,
                      std::string_view expected) {
    const auto info = archive.resolve_file(snapshot, path);
    if (!check(info && info.value->size == expected.size(), "writer output text file size")) return false;
    std::vector<std::uint8_t> out(expected.size());
    const auto status = archive.read_file(info.value->id, 0, out.size(), out.data(), out.size());
    return check(status.ok() && std::string_view(reinterpret_cast<const char*>(out.data()), out.size()) == expected,
                 "writer output text content");
}

static bool read_fill(nx::Archive& archive, std::uint32_t snapshot, std::string_view path,
                      std::uint64_t size, std::uint64_t offset, std::size_t length,
                      std::uint8_t value) {
    const auto info = archive.resolve_file(snapshot, path);
    if (!check(info && info.value->size == size, "dedup file resolves with expected size")) return false;
    std::vector<std::uint8_t> out(length, 0);
    const auto result = archive.read_file(info.value->id, offset, length, out.data(), out.size());
    return check(result.ok() && std::all_of(out.begin(), out.end(), [value](std::uint8_t byte) {
                     return byte == value;
                 }),
                 "deduplicated block random read has expected bytes");
}

int main(int argc, char** argv) {
    if (argc != 3) return 2;
    FileSource source(argv[1]);
    if (!check(source.good(), "open writer archive file")) return 1;
    RealZstd zstd;
    if (!check(zstd.ready(), "load real Zstandard runtime for Rust writer output")) return 1;
    nx::OpenOptions options;
    options.zstd = &RealZstd::decode;
    options.zstd_context = &zstd;
    auto opened = nx::Archive::open(source, options);
    if (std::string(argv[2])=="reject_open") return opened ? 1 : 0;
    if (!check(opened.ok(), "C++ reader open() on Rust output")) return 1;
    nx::Archive archive = std::move(*opened.value);
    if (std::string(argv[2]) == "metadata") {
        if (!check(archive.title(0) == "Current native title" &&
                   archive.publisher(0) == "Current publisher" &&
                   archive.localized_entries().size() == 1 &&
                   archive.localized_language(0) == "fr" &&
                   archive.applications()[0].metadata_snapshot == 1,
                   "Fast Catalog selected metadata is available after open()")) return 1;
        const auto media = archive.read_media(archive.applications()[0].icon_index);
        if (!check(media && std::string_view(reinterpret_cast<const char*>(media.value->data()), media.value->size()) == "current JPEG bytes",
                   "on-demand native icon bytes before mount")) return 1;
    }
    if (!check(archive.mount().ok(), "C++ reader mount() on Rust output")) return 1;
    const std::string kind = argv[2];
    if (kind == "reject_verify") {
        if (archive.verify_all()) {
            std::cerr << "FAIL corrupted Rust output passed C++ reader verify_all()\n";
            return 1;
        }
        std::cout << "PASS C++ reader rejected corrupted Zstandard output during verify_all()\n";
        return 0;
    }
    if (!check(archive.verify_all().ok(), "C++ reader verify_all() on Rust output")) return 1;
    if (kind == "raw") {
        constexpr std::uint64_t size=700123;
        for (const auto offset : {std::uint64_t(0), std::uint64_t(262137), std::uint64_t(262144), std::uint64_t(524281), size-193})
            if (!read_pattern(archive,0,"folder/big.bin",size,offset)) return 1;
        if (!read_text(archive,0,"note.txt","NX writer stream test")) return 1;
    } else if (kind == "metadata") {
        if (!check(archive.snapshots()[0].required_firmware==100u && archive.snapshots()[1].required_firmware==200u &&
                   archive.snapshots()[0].npdm_index!=archive.snapshots()[1].npdm_index,"versioned native metadata")) return 1;
        if (!read_text(archive,0,"shared.bin","stable file across complete snapshots") ||
            !read_text(archive,0,"removed.bin","removed in update") ||
            !read_text(archive,1,"shared.bin","stable file across complete snapshots") ||
            !read_text(archive,1,"new.bin","added in update")) return 1;
        auto root=archive.root_directory(2);
        unsigned count=0;
        if (!root || !archive.enumerate(*root.value,[&](const nx::DirectoryEntryView&){++count;return true;}).ok() || count!=0)
            return check(false,"unassociated DLC snapshot is a complete empty tree") ? 0 : 1;
    } else if (kind == "dlc") {
        if (!check(archive.programs().empty() && archive.snapshots().size()==1 && archive.snapshots()[0].program_index==0xffffffffu,
                   "DLC-only application does not invent a Program row") ||
            !read_text(archive,0,"dlc.bin","unassociated DLC tree")) return 1;
    } else if (kind == "programs") {
        if (!check(archive.programs().size()==2 && archive.snapshots().size()==2,
                   "multiple executable Program rows and snapshots" ) ||
            !read_text(archive,0,"first.bin","first executable tree") ||
            !read_text(archive,1,"second.bin","second executable tree") ||
            !read_text(archive,1,"shared.bin","first executable tree")) return 1;
    } else if (kind == "global_dedup") {
        constexpr std::uint64_t chunk = 256 * 1024;
        if (!check(archive.snapshots().size() == 3 && archive.applications().size() == 2,
                   "global dedup spans snapshots and applications") ||
            !read_fill(archive, 0, "a.bin", 2 * chunk, chunk - 64, 64, 0xa1) ||
            !read_fill(archive, 0, "a.bin", 2 * chunk, chunk, 193, 0xb2) ||
            !read_fill(archive, 1, "changed.bin", 2 * chunk, 0, 193, 0xa1) ||
            !read_fill(archive, 1, "changed.bin", 2 * chunk, chunk, 193, 0xd4) ||
            !read_fill(archive, 1, "copy.bin", 2 * chunk, chunk, 193, 0xb2) ||
            !read_fill(archive, 2, "cross.bin", chunk, 0, 193, 0xa1) ||
            !read_fill(archive, 1, "tail.bin", 137, 0, 137, 0xe5)) return 1;
        const auto empty = archive.resolve_file(0, "empty.bin");
        if (!check(empty && empty.value->size == 0, "empty file remains intact with dedup")) return 1;
    } else if (kind == "zstd_only") {
        constexpr std::uint64_t size = 2 * 256 * 1024 + 128 * 1024;
        if (!read_fill(archive, 0, "compressible.bin", size, 256 * 1024 - 31, 193, 0x5a)) return 1;
    } else if (kind == "mixed_codecs") {
        constexpr std::uint64_t chunk = 256 * 1024;
        constexpr std::uint64_t size = 2 * chunk + chunk / 2;
        if (!read_fill(archive, 0, "compressible.bin", size, chunk - 32, 193, 0x5a)) return 1;
        auto noise = archive.resolve_file(0, "incompressible.bin");
        std::array<std::uint8_t, 32> output{};
        if (!check(noise && noise.value->size == chunk &&
                   archive.read_file(noise.value->id, 0, output.size(), output.data(), output.size()).ok(),
                   "mixed RAW noise block random read")) return 1;
        std::uint32_t state = 0x13579bdf;
        std::array<std::uint8_t, 32> expected{};
        for (std::size_t i = 0; i < expected.size(); i += 4) {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            const auto word = state;
            for (std::size_t b = 0; b < 4 && i + b < expected.size(); ++b)
                expected[i + b] = static_cast<std::uint8_t>(word >> (8 * b));
        }
        if (!check(output == expected, "mixed RAW noise bytes match the deterministic source")) return 1;
    } else if (kind.rfind("chunk_", 0) == 0) {
        const auto kib = std::stoul(kind.substr(6));
        const std::uint64_t chunk = kib * 1024;
        const std::uint64_t size = 2 * chunk + 37;
        if (!read_fill(archive, 0, "chunk.bin", size, chunk - 8, 32, 0x6b)) return 1;
    } else if (kind=="removed_update" || kind=="removed_dlc" || kind=="removed_base" || kind=="added_update" || kind=="added_dlc") {
        const bool old=kind=="removed_update", third=kind=="added_update", no_base=kind=="removed_base";
        const auto expected_title=old?"Original native title":third?"Third version title":"Current native title";
        const auto selected=old||no_base?0u:third?2u:1u;
        if (!check(archive.title(0)==expected_title && archive.applications()[0].metadata_snapshot==selected,
                   "snapshot edit did not reselect correct native display metadata")) return 1;
        const auto& snap=archive.snapshots()[selected];
        if (!check(snap.required_firmware && *snap.required_firmware==(old?100u:third?300u:200u),"snapshot firmware lost after edit")) return 1;
        auto npdm=archive.read_media(snap.npdm_index);
        const auto expected_npdm=old?"base NPDM":third?"third NPDM":"update NPDM";
        if (!check(npdm && std::string(npdm.value->begin(),npdm.value->end())==expected_npdm,"NPDM belongs to another version after edit")) return 1;
        if (old) {
            if (archive.resolve_file(0,"new.bin") || !read_text(archive,0,"removed.bin","removed in update")) return 1;
            auto icon=archive.read_media(archive.applications()[0].icon_index);
            auto nacp=archive.read_media(archive.applications()[0].nacp_index);
            if (!check(archive.publisher(0)=="Original publisher" && archive.localized_language(0)=="ja" &&
                       icon && std::string(icon.value->begin(),icon.value->end())=="original PNG bytes" &&
                       nacp && std::string(nacp.value->begin(),nacp.value->end())=="synthetic base NACP",
                       "removing selected update lost historical native display metadata")) return 1;
        } else if (!third && !read_text(archive,selected,"new.bin","added in update")) return 1;
        if (third && !read_text(archive,selected,"future.bin","third version")) return 1;
        if (kind=="added_dlc" && !read_text(archive,3,"extra.bin","new DLC content")) return 1;
    } else if (kind != "empty") return 2;
    std::cout << "PASS C++17 open, mount, verify_all, metadata and RAW/Zstandard random reads: " << kind << '\n';
    return 0;
}
