#include "nx_native_sha256.hpp"
#include <atomic>
#include <iostream>
#include <thread>
#include <vector>
int main() {
    const std::array<std::uint8_t,32> empty{0xe3,0xb0,0xc4,0x42,0x98,0xfc,0x1c,0x14,
        0x9a,0xfb,0xf4,0xc8,0x99,0x6f,0xb9,0x24,0x27,0xae,0x41,0xe4,0x64,0x9b,0x93,
        0x4c,0xa4,0x95,0x99,0x1b,0x78,0x52,0xb8,0x55};
    const std::array<std::uint8_t,32> million_a{0xcd,0xc7,0x6e,0x5c,0x99,0x14,0xfb,0x92,
        0x81,0xa1,0xc7,0xe2,0x84,0xd7,0x3e,0x67,0xf1,0x80,0x9a,0x48,0xa4,0x97,0x20,
        0x0e,0x04,0x6d,0x39,0xcc,0xc7,0x11,0x2c,0xd0};
    std::array<std::uint8_t,32> output{};
    if (!nx::native_block_sha256(nullptr,nullptr,0,output.data()) || output!=empty) return 1;
    std::vector<std::uint8_t> data(1024*1024,'a');
    if (!nx::native_block_sha256(nullptr,data.data(),1000000,output.data()) || output!=million_a) return 1;
    if (nx::native_block_sha256(nullptr,nullptr,1,output.data()) ||
        nx::native_block_sha256(nullptr,data.data(),data.size()+1,output.data()) ||
        nx::native_block_sha256(nullptr,data.data(),data.size(),nullptr)) return 1;
    for (std::size_t i=0;i<data.size();++i) data[i]=static_cast<std::uint8_t>(i*37+i/257);
    std::atomic<unsigned> errors{0};
    std::vector<std::thread> threads;
    for (unsigned t=0;t<8;++t) threads.emplace_back([&] {
        std::array<std::uint8_t,32> digest{};
        for (auto size:{1u,55u,56u,63u,64u,65u,4096u,65536u,131072u,262144u,524288u,1048576u})
            if (!nx::native_block_sha256(nullptr,data.data(),size,digest.data()) ||
                digest!=nx::Sha256::digest(data.data(),size)) ++errors;
    });
    for (auto& t:threads) t.join();
    if (errors) return 1;
    std::cout<<"PASS "<<nx::native_sha256_name()<<": independent vectors, padding/block boundaries, concurrency and bounded arguments\n";
}
