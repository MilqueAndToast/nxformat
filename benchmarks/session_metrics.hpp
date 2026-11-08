#pragma once
#include <atomic>
#include <cstdlib>
#include <new>
#include <chrono>
#include <condition_variable>
#include <mutex>
#include <thread>
#if defined(_WIN32)
#include <windows.h>
#include <psapi.h>
#pragma comment(lib,"psapi.lib")
#else
#include <sys/resource.h>
#include <unistd.h>
#include <fstream>
#if defined(__APPLE__)
#include <mach/mach.h>
#endif
#endif
// Benchmark-only counters; C/OpenSSL/Zstd internal allocations are not counted.
inline std::atomic<std::uint64_t> cpp_allocations{0},cpp_allocated_bytes{0},cpp_allocation_ns{0};
inline std::atomic<bool> cpp_time_allocations{false};
#if defined(__GNUC__)
#define NX_BENCH_NOINLINE __attribute__((noinline))
#elif defined(_MSC_VER)
#define NX_BENCH_NOINLINE __declspec(noinline)
#else
#define NX_BENCH_NOINLINE
#endif
NX_BENCH_NOINLINE void* operator new(std::size_t size) {
    const bool timed=cpp_time_allocations.load(std::memory_order_relaxed);
    const auto start=timed ? std::chrono::steady_clock::now() : std::chrono::steady_clock::time_point{};
    if (auto p=std::malloc(size ? size : 1)) {
        if (timed) cpp_allocation_ns+=static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::nanoseconds>(std::chrono::steady_clock::now()-start).count());
        ++cpp_allocations; cpp_allocated_bytes+=size; return p;
    }
    throw std::bad_alloc();
}
NX_BENCH_NOINLINE void* operator new[](std::size_t size) { return ::operator new(size); }
NX_BENCH_NOINLINE void operator delete(void* p) noexcept { std::free(p); }
NX_BENCH_NOINLINE void operator delete[](void* p) noexcept { std::free(p); }
NX_BENCH_NOINLINE void operator delete(void* p,std::size_t) noexcept { std::free(p); }
NX_BENCH_NOINLINE void operator delete[](void* p,std::size_t) noexcept { std::free(p); }
struct ProcessMetrics { std::uint64_t rss=0,peak=0; double cpu_s=0; };
inline ProcessMetrics process_metrics() {
    ProcessMetrics result;
#if defined(_WIN32)
    PROCESS_MEMORY_COUNTERS memory{}; memory.cb=sizeof(memory);
    if (GetProcessMemoryInfo(GetCurrentProcess(),&memory,sizeof(memory))) {
        result.rss=memory.WorkingSetSize; result.peak=memory.PeakWorkingSetSize;
    }
    FILETIME create{},exit{},kernel{},user{};
    if (GetProcessTimes(GetCurrentProcess(),&create,&exit,&kernel,&user)) {
        auto seconds=[](FILETIME v){ return double((std::uint64_t(v.dwHighDateTime)<<32)|v.dwLowDateTime)/10000000.; };
        result.cpu_s=seconds(kernel)+seconds(user);
    }
#else
    rusage usage{};
    if (!getrusage(RUSAGE_SELF,&usage)) {
        result.cpu_s=double(usage.ru_utime.tv_sec+usage.ru_stime.tv_sec)+
            double(usage.ru_utime.tv_usec+usage.ru_stime.tv_usec)/1000000.;
        result.peak=static_cast<std::uint64_t>(usage.ru_maxrss);
#if !defined(__APPLE__)
        result.peak*=1024;
#endif
    }
#if defined(__APPLE__)
    mach_task_basic_info info{}; mach_msg_type_number_t count=MACH_TASK_BASIC_INFO_COUNT;
    if (task_info(mach_task_self(),MACH_TASK_BASIC_INFO,reinterpret_cast<task_info_t>(&info),&count)==KERN_SUCCESS)
        result.rss=info.resident_size;
#else
    std::ifstream statm("/proc/self/statm"); std::uint64_t total=0,resident=0;
    if (statm>>total>>resident) result.rss=resident*static_cast<std::uint64_t>(sysconf(_SC_PAGESIZE));
#endif
#endif
    return result;
}

// Opt-in benchmark instrumentation. CPU/allocations include the sampler's
// small overhead; endpoints and sample count expose very short-run limits.
class RssSampler {
    using Clock=std::chrono::steady_clock;
    std::mutex mutex_;
    std::condition_variable cv_;
    bool stopped_=false;
    std::thread worker_;
    Clock::time_point last_=Clock::now();
    std::uint64_t rss_=0;
    long double integral_=0;
    double seconds_=0;
    void observe() {
        const auto now=Clock::now();
        const auto elapsed=std::chrono::duration<double>(now-last_).count();
        integral_+=static_cast<long double>(rss_)*elapsed;
        seconds_+=elapsed; last_=now; rss_=process_metrics().rss; ++samples;
    }
public:
    std::uint64_t samples=0;
    explicit RssSampler(unsigned interval_ms) {
        if (!interval_ms) return;
        rss_=process_metrics().rss; samples=1; last_=Clock::now();
        worker_=std::thread([this,interval_ms] {
            std::unique_lock<std::mutex> lock(mutex_);
            while (!cv_.wait_for(lock,std::chrono::milliseconds(interval_ms),[&]{return stopped_;})) observe();
            observe();
        });
    }
    void stop() {
        if (!worker_.joinable()) return;
        { std::lock_guard<std::mutex> lock(mutex_); stopped_=true; }
        cv_.notify_all(); worker_.join();
    }
    double mean() const { return seconds_>0 ? static_cast<double>(integral_/seconds_) : 0; }
    ~RssSampler() { stop(); }
};
