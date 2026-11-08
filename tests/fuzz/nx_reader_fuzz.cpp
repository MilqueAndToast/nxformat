// Build with clang++ -std=c++17 -fsanitize=fuzzer,address,undefined.
#include "nx_reader.hpp"

#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <vector>

class FuzzSource final : public nx::RandomAccessSource {
public:
    explicit FuzzSource(const std::vector<std::uint8_t>& input) : input_(input) {}
    std::uint64_t size() const noexcept override { return input_.size(); }
    bool read_at(std::uint64_t offset,std::uint8_t* output,std::size_t length) override {
        if (offset>input_.size() || length>input_.size()-offset) return false;
        if (length) std::copy(input_.begin()+static_cast<std::size_t>(offset),input_.begin()+static_cast<std::size_t>(offset+length),output);
        return true;
    }
private:
    const std::vector<std::uint8_t>& input_;
};

extern "C" int LLVMFuzzerTestOneInput(const std::uint8_t* data,std::size_t size) {
    if (!size || size>16*1024*1024) return 0;
    std::vector<std::uint8_t> bytes(data,data+size);
    FuzzSource source(bytes);
    nx::OpenOptions options;
    options.max_manifest_bytes=8*1024*1024;
    options.max_directories=100'000;
    options.max_files=100'000;
    options.max_entries=200'000;
    options.max_block_references=500'000;
    options.max_blocks=100'000;
    options.max_snapshot_nodes=200'000;
    auto opened=nx::Archive::open(source,options);
    if (opened) { nx::Archive archive=std::move(*opened.value); (void)archive.mount(); }
    return 0;
}
