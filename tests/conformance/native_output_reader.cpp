#include "nx_reader.hpp"
#include "nx_native_sha256.hpp"
#include "real_zstd.hpp"

#include <cstdint>
#include <filesystem>
#include <sstream>
#include <vector>
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
    if (argc != 3 && argc != 4) return 2;
    FileSource source(argv[1]);
    RealZstd zstd;
    if (!source.good() || !zstd.ready()) return 2;
    nx::OpenOptions options;
    options.zstd = &RealZstd::decode; options.zstd_context = &zstd;
    options.block_sha256=&nx::native_block_sha256;
    auto opened = nx::Archive::open(source, options);
    if (!opened) { std::cerr << opened.error.message; return 1; }
    auto archive = std::move(*opened.value);
    if (!archive.mount() || !archive.verify_all()) return 1;
    if (argc == 4) {
        std::ifstream selections(std::filesystem::u8path(argv[3]));
        if (!selections) return 2;
        std::string row;
        std::size_t checked = 0;
        while (std::getline(selections,row)) {
            std::vector<std::string> columns;std::istringstream split(row);std::string column;
            while(std::getline(split,column,'\t')) columns.push_back(column);
            if(columns.size()==6) {
                // native_convert: snapshot, application, Program, version,
                // canonical identity, Build ID. Preserve the older four-column
                // native-add selection contract below as well.
                const auto n=static_cast<std::size_t>(std::stoull(columns[0]));
                if(n>=archive.snapshots().size()) return 1;
                const auto& s=archive.snapshots()[n];
                const nx::Application* app=nullptr;
                for(const auto& a:archive.applications()) if(n>=a.first_snapshot && n-a.first_snapshot<a.snapshot_count) app=&a;
                if(!app || app->id!=std::stoull(columns[1],nullptr,16) || s.numeric_version!=std::stoul(columns[3])) return 1;
                if(columns[2]=="-") {if(s.program_index!=0xffffffffu)return 1;}
                else if(s.program_index>=archive.programs().size() || archive.programs()[s.program_index].id!=std::stoull(columns[2],nullptr,16))return 1;
                const auto check_hex=[](const std::string& text,const auto& bytes) {
                    if(text=="-") return std::all_of(bytes.begin(),bytes.end(),[](auto b){return b==0;});
                    if(text.size()!=bytes.size()*2) return false;
                    for(std::size_t i=0;i<bytes.size();++i) if(bytes[i]!=std::stoul(text.substr(i*2,2),nullptr,16)) return false;
                    return true;
                };
                if(!check_hex(columns[4],s.identity) || !check_hex(columns[5],s.build_id))return 1;
                ++checked;continue;
            }
            std::istringstream fields(row); std::string index,program,build,nacp;
            if (!std::getline(fields,index,'\t') || !std::getline(fields,program,'\t') || !std::getline(fields,build,'\t') || !std::getline(fields,nacp)) return 2;
            const auto n=static_cast<std::size_t>(std::stoull(index));
            if (n>=archive.snapshots().size()) return 1;
            const auto& snapshot=archive.snapshots()[n];
            if (program=="-") {if(snapshot.program_index!=0xffffffffu) return 1;}
            else if(snapshot.program_index>=archive.programs().size() || archive.programs()[snapshot.program_index].id!=std::stoull(program,nullptr,16)) return 1;
            if (build=="-") {if(std::any_of(snapshot.build_id.begin(),snapshot.build_id.end(),[](auto byte){return byte!=0;})) return 1;}
            else {
                if(build.size()!=64) return 2;
                for(std::size_t i=0;i<32;++i) if(snapshot.build_id[i]!=std::stoul(build.substr(i*2,2),nullptr,16)) return 1;
            }
            if(nacp=="-") {if(snapshot.nacp_index!=0xffffffffu) return 1;}
            else {
                auto payload=archive.read_media(snapshot.nacp_index);if(!payload || nacp.size()!=64) return 1;
                const auto hash=nx::Sha256::digest(payload.value->data(),payload.value->size());
                for(std::size_t i=0;i<32;++i) if(hash[i]!=std::stoul(nacp.substr(i*2,2),nullptr,16)) return 1;
            }
            ++checked;
        }
        if(checked!=archive.snapshots().size()) return 1;
        std::cout << "PASS C++17 explicit Program/Build ID selection: " << checked << " snapshots\n";
    }
    std::ifstream expected(std::filesystem::u8path(argv[2]));
    if (!expected) return 2;
    std::string line;
    std::vector<std::uint8_t> native(262144), packed(262144);
    unsigned count = 0;
    while (std::getline(expected,line)) {
        std::istringstream row(line);
        std::string index, path, container, offset, length;
        if (!std::getline(row,index,'\t') || !std::getline(row,path,'\t') ||
            !std::getline(row,container,'\t') || !std::getline(row,offset,'\t') || !std::getline(row,length)) return 2;
        const auto snapshot = static_cast<std::uint32_t>(std::stoul(index));
        const auto size = std::stoull(length);
        auto info = archive.resolve_file(snapshot,path);
        if (!info || info.value->size != size) return 1;
        std::ifstream original(std::filesystem::u8path(container),std::ios::binary);
        original.seekg(static_cast<std::streamoff>(std::stoull(offset)));
        for (std::uint64_t at=0; at<size;) {
            const auto n = static_cast<std::size_t>(std::min<std::uint64_t>(native.size(),size-at));
            original.read(reinterpret_cast<char*>(native.data()),static_cast<std::streamsize>(n));
            if (original.gcount()!=static_cast<std::streamsize>(n) ||
                !archive.read_file(info.value->id,at,n,packed.data(),packed.size()) ||
                !std::equal(native.begin(),native.begin()+n,packed.begin())) {
                std::cerr << "native content mismatch " << path; return 1;
            }
            at+=n;
        }
        ++count;
    }
    if (!count) return 1;
    std::cout << "PASS C++17 native byte comparisons: " << count << " files; full verification\n";
    return 0;
}
