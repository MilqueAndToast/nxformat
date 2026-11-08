// Reproducible, verified access-pattern timings for the actual persistent
// reader. No benchmark-only archive implementation and no proprietary data.
#include "nx_disk_source.hpp"
#include "nx_session.hpp"
#include "nx_native_sha256.hpp"
#include "real_zstd.hpp"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <condition_variable>
#include <cstdint>
#include <fstream>
#include <filesystem>
#include <iomanip>
#include <iostream>
#include <limits>
#include <mutex>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>
#include "session_metrics.hpp"
#include "loose_cache.hpp"
#if defined(NX_BENCH_NXEMU)
#include "core/file_sys/vfs/vfs_nx.h"
class HostPlaceholder final : public FileSys::VfsFile {
public:
    std::string GetName() const override { return "host.nx"; }
    size_t GetSize() const override { return 0; }
    FileSys::VirtualDir GetContainingDirectory() const override { return {}; }
    bool IsWritable() const override { return false; }
    bool IsReadable() const override { return true; }
    bool Resize(size_t) override { return false; }
    bool Rename(std::string_view) override { return false; }
    size_t Write(const u8*,size_t,size_t) override { return 0; }
    size_t Read(u8*,size_t,size_t) const override { return 0; }
};
#endif

namespace {
class LooseTree final : public nx::RandomAccessSource {
public:
    std::vector<std::shared_ptr<nx::DiskSource>> files;
    std::uint64_t part_size=0;
    std::uint64_t size() const noexcept override { return part_size*files.size(); }
    bool read_at(std::uint64_t at,std::uint8_t* output,std::size_t count) override {
        if (at>size() || count>size()-at) return false;
        auto* bytes=static_cast<std::uint8_t*>(output);
        while (count) {
            const auto index=static_cast<std::size_t>(at/part_size);
            const auto offset=at%part_size;
            const auto n=static_cast<std::size_t>(std::min<std::uint64_t>(count,part_size-offset));
            if (!files[index]->read_at(offset,bytes,n)) return false;
            count-=n; at+=n; bytes+=n;
        }
        return true;
    }
};
std::atomic<std::uint64_t> decoder_bytes{0},decoder_peak{0},decoder_contexts{0};
struct Sample {
    std::uint64_t offset=0;
    std::size_t length=0;
    double latency_us=0;
};
struct Worker {
    std::vector<Sample> samples;
    std::uint64_t bytes=0, checksum=14695981039346656037ull;
    bool failed=false;
};
std::uint64_t random_next(std::uint64_t& state) {
    state+=0x9e3779b97f4a7c15ull;
    auto v=state;
    v=(v^(v>>30))*0xbf58476d1ce4e5b9ull;
    v=(v^(v>>27))*0x94d049bb133111ebull;
    return v^(v>>31);
}
bool decode(void*,const std::uint8_t* input,std::size_t size,std::uint8_t* output,
            std::size_t capacity,std::size_t* written,std::size_t* consumed) {
    // Test decoder only; actual NXEmu mode installs its session-owned pool.
    struct TrackedDecoder {
        RealZstd zstd;
        std::size_t bytes=0;
        TrackedDecoder() { ++decoder_contexts; }
        ~TrackedDecoder() { decoder_bytes-=bytes; --decoder_contexts; }
        void account() {
            const auto current=zstd.context_bytes();
            const auto total=current>=bytes ? decoder_bytes.fetch_add(current-bytes)+current-bytes :
                decoder_bytes.fetch_sub(bytes-current)-(bytes-current);
            bytes=current;
            auto peak=decoder_peak.load();
            while (peak<total && !decoder_peak.compare_exchange_weak(peak,total)) {}
        }
    };
    thread_local TrackedDecoder decoder;
    const auto result=RealZstd::decode(&decoder.zstd,input,size,output,capacity,written,consumed);
    decoder.account(); return result;
}
double percentile(const std::vector<double>& sorted,double fraction) {
    const auto index=static_cast<std::size_t>(std::ceil(fraction*static_cast<double>(sorted.size())));
    return sorted[std::min(sorted.size()-1,std::max<std::size_t>(1,index)-1)];
}
Sample next_sample(const std::string& mode,std::uint64_t file_size,
                   std::uint64_t block_size,std::uint64_t& random_state,
                   std::uint64_t& sequential,std::size_t iteration) {
    const std::size_t read_size=
        mode=="uniform1m" ? 1024*1024 :
        mode=="uniform64k" ? 64*1024 :
        mode=="gameload" ? (iteration%16<4 ? 512 : iteration%16<12 ? 64*1024 : 4096) :
        mode=="sequential1m" ? 1024*1024 :
        mode=="sequential64k" ? 64*1024 :
        mode=="mixed" && (iteration%4==0) ? 64*1024 : 4096;
    const auto length=static_cast<std::size_t>(std::min<std::uint64_t>(file_size,read_size));
    const auto max_offset=file_size-length;
    std::uint64_t offset=0;
    if (mode=="scatter4k" || mode=="gameload") {
        const auto part=file_size/16;
        const auto index=random_next(random_state)%16;
        if (mode=="gameload" && iteration%16<4) offset=index*part+(iteration%4)*512;
        else if (mode=="gameload" && iteration%16<12)
            offset=index*part+((iteration%8)*65536)%(part-length+1);
        else offset=index*part+(random_next(random_state)%(part-length+1))/4096*4096;
    } else if (mode=="sequential64k" || mode=="sequential1m" ||
        (mode=="mixed" && iteration%4==0)) {
        if (sequential>max_offset) sequential=0;
        offset=sequential;
        sequential+=length;
    } else if (mode=="hot4k" || mode=="mixed") {
        const auto hot=std::min<std::uint64_t>(file_size,4ull*1024*1024);
        offset=(random_next(random_state) % (hot-length+1))/4096*4096;
    } else if (mode=="cross4k" && file_size>block_size) {
        const auto boundaries=(file_size-1)/block_size;
        const auto block=1+random_next(random_state)%boundaries;
        offset=std::min(max_offset,block*block_size-std::min<std::uint64_t>(2048,block_size));
    } else {
        offset=(random_next(random_state) % (max_offset+1))/4096*4096;
    }
    return {offset,length,0};
}
void update_checksum(std::uint64_t& state,const std::uint8_t* bytes,std::size_t length) {
    for (std::size_t i=0;i<length;++i) {
        state^=bytes[i];
        state*=1099511628211ull;
    }
}
} // namespace

#include "captured_trace_replay.hpp"

int main(int argc,char** argv) {
    if(argc>1 && std::string(argv[1])=="--captured") {
        try {return replay_captured(argc,argv);} catch(const std::exception& error) {std::cerr<<error.what()<<'\n';return 1;}
    }
    if (argc<12 || argc>15) {
        std::cerr<<"usage: session_trace_benchmark archive.nx profile codec block_kib "
                    "trace cache_mib prefetch_0_or_1 samples_per_thread threads seed sample_csv_or_dash [portable|native] [cold|replay] [rss_interval_ms]\n"
                    "traces: uniform4k uniform64k uniform1m hot4k cross4k sequential64k sequential1m mixed scatter4k gameload\n";
        return 2;
    }
    try {
        const std::string profile=argv[2],codec=argv[3],mode=argv[5],sample_path=argv[11];
        const auto block_kib=std::stoull(argv[4]);
        const auto cache_mib=std::stoull(argv[6]);
        const auto prefetch=std::stoull(argv[7]);
        const auto per_thread=std::stoull(argv[8]);
        const auto thread_count=std::stoull(argv[9]);
        const auto seed=std::stoull(argv[10],nullptr,0);
        const std::string sha=argc>12 ? argv[12] : "portable";
        const std::string phase=argc>13 ? argv[13] : "cold";
        const auto rss_interval=argc>14 ? std::stoul(argv[14]) : 0;
        if (rss_interval>1000) throw std::invalid_argument("RSS interval must be 0..1000 ms");
        if ((sha!="portable" && sha!="native") || (phase!="cold" && phase!="replay"))
            throw std::invalid_argument("unsupported hash provider/trace phase");
        if (block_kib<64 || block_kib>1024 || cache_mib>256 || prefetch>1 ||
            per_thread<1000 || per_thread>1000000 || thread_count<1 || thread_count>8 ||
            (mode!="uniform4k" && mode!="hot4k" && mode!="cross4k" &&
             mode!="sequential64k" && mode!="sequential1m" && mode!="mixed" &&
             mode!="uniform64k" && mode!="uniform1m" && mode!="scatter4k" && mode!="gameload"))
            throw std::invalid_argument("unsupported benchmark configuration");
        if (codec!="loose" && codec!="loose-cached" && codec!="raw" && codec!="zstd")
            throw std::invalid_argument("unsupported codec");
        const auto process_baseline=process_metrics();
        const bool multi=mode=="scatter4k" || mode=="gameload";
        std::shared_ptr<LooseTree> loose_tree;
        std::shared_ptr<nx::RandomAccessSource> disk;
#if defined(NX_BENCH_NXEMU)
        if (codec=="loose" || codec=="loose-cached")
#endif
        {
            auto disk_result=nx::DiskSource::open(argv[1]);
            if (!disk_result) throw std::runtime_error("cannot open archive: "+disk_result.error.message);
            disk=*disk_result.value;
            if (multi) {
                loose_tree=std::make_shared<LooseTree>();
                const auto base=std::filesystem::path(argv[1]);
                for (unsigned i=0;i<16;++i) {
                    auto name=base.parent_path()/(base.stem().string()+".part"+(i<10 ? "0" : "")+std::to_string(i)+".bin");
                    auto part=nx::DiskSource::open(name);
                    if (!part || !(*part.value)->size()) throw std::runtime_error("loose tree part missing");
                    if (!i) loose_tree->part_size=(*part.value)->size();
                    if ((*part.value)->size()!=loose_tree->part_size) throw std::runtime_error("loose tree size mismatch");
                    loose_tree->files.push_back(*part.value);
                }
                disk=loose_tree;
            }
        }
        std::shared_ptr<nx::Session> session;
        std::unique_ptr<LooseCache> loose_cache;
        nx::FileId file_id{};
        std::vector<nx::FileId> file_ids;
        std::uint64_t file_size=disk ? disk->size() : 0;
        std::uint64_t chunk_size=block_kib*1024;
        if (codec=="raw" || codec=="zstd") {
            nx::SessionOptions settings;
            settings.reader.zstd=&decode;
            settings.reader.block_sha256=sha=="native" ? &nx::native_block_sha256 : &nx::portable_block_sha256;
            settings.reader.max_cached_block_bytes=cache_mib*1024*1024;
            settings.reader.max_cached_blocks=cache_mib ? 1024 : 0;
            settings.reader.max_read_bytes=1024*1024;
            settings.parallel_reads=static_cast<std::size_t>(thread_count)+1;
            settings.sequential_prefetch=bool(prefetch);
#if defined(NX_BENCH_NXEMU)
            auto session_result=FileSys::OpenNxSession(std::make_shared<HostPlaceholder>(),argv[1],settings);
#else
            auto session_result=nx::Session::open(disk,settings);
#endif
            if (!session_result) throw std::runtime_error("cannot mount archive: "+session_result.error.message);
            session=*session_result.value;
#if defined(NX_BENCH_NXEMU)
            // The host-path branch pins exactly one DiskSource. Borrow its
            // counters rather than reopening the archive for instrumentation.
            disk=std::shared_ptr<nx::RandomAccessSource>(session,&session->backing_source());
#endif
            auto file=session->archive().resolve_file(0,multi ? "payload00.bin" : "payload.bin");
            if (!file) throw std::runtime_error("benchmark archive lacks payload.bin");
            auto info=session->archive().file_info(file.value->id);
            if (!info || !info.value->size) throw std::runtime_error("benchmark file is empty");
            file_id=file.value->id;
            file_size=info.value->size;
            if (multi) {
                const auto part_size=file_size;
                for (unsigned i=0;i<16;++i) {
                    auto path="payload"+std::string(i<10 ? "0" : "")+std::to_string(i)+".bin";
                    auto part=session->archive().resolve_file(0,path);
                    if (!part) throw std::runtime_error("NX tree part missing");
                    auto details=session->archive().file_info(part.value->id);
                    if (!details || details.value->size!=part_size) throw std::runtime_error("NX tree size mismatch");
                    file_ids.push_back(part.value->id);
                }
                file_size*=16;
            } else file_ids.push_back(file_id);
            chunk_size=1ull<<info.value->chunk_log2;
            if (chunk_size!=block_kib*1024)
                throw std::runtime_error("declared block size differs from benchmark metadata");
        } else if (codec=="loose-cached" && file_size && cache_mib && !prefetch) {
            loose_cache=std::make_unique<LooseCache>(disk,chunk_size,cache_mib*1024*1024);
        } else if (!file_size || cache_mib || prefetch) {
            throw std::invalid_argument("loose reference requires cache=0 prefetch=0");
        }
        // Explicit warm-OS policy, through the very same pinned source. This
        // never populates NX/loose application caches. It is outside timing.
        {
            std::vector<std::uint8_t> warm(1024*1024);
            for (std::uint64_t at=0;at<disk->size();) {
                const auto size=static_cast<std::size_t>(std::min<std::uint64_t>(warm.size(),disk->size()-at));
                if (!disk->read_at(at,warm.data(),size)) throw std::runtime_error("OS warmup failed");
                at+=size;
            }
        }
        const auto idle=process_metrics();
        std::uint64_t metadata_wire_bytes=0;
        if (session) {
            std::array<std::uint8_t,112> header{};
            if (!disk->read_at(0,header.data(),header.size())) throw std::runtime_error("metadata accounting header read");
            auto u64=[&](std::size_t at) { std::uint64_t v=0; for (unsigned i=0;i<8;++i) v|=std::uint64_t(header[at+i])<<(8*i); return v; };
            metadata_wire_bytes=u64(32)+u64(80)+u64(96)*56; // Catalog, Manifest and authenticated block descriptors+hashes.
        }
        std::vector<Worker> results(static_cast<std::size_t>(thread_count));
        std::vector<std::thread> threads;
        std::mutex start_mutex;
        std::condition_variable start_cv;
        std::size_t ready=0;
        bool go=false;
        for (std::size_t worker=0;worker<thread_count;++worker) {
            threads.emplace_back([&,worker] {
                auto& result=results[worker];
                std::vector<std::uint8_t> bytes;
                std::shared_ptr<nx::ReadCursor> cursor;
#if defined(NX_BENCH_NXEMU)
                std::vector<FileSys::VirtualFile> vfs;
#endif
                try {
                    result.samples.reserve(static_cast<std::size_t>(per_thread));
                    bytes.resize(1024*1024);
                    cursor=std::make_shared<nx::ReadCursor>();
#if defined(NX_BENCH_NXEMU)
                    if (session) {
                        auto directory=FileSys::NxSnapshotDirectory(session,0);
                        for (unsigned i=0;i<(multi ? 16u : 1u);++i) {
                            const auto name=multi ? "payload"+std::string(i<10 ? "0" : "")+std::to_string(i)+".bin" : "payload.bin";
                            auto file=directory ? directory->GetFile(name) : nullptr;
                            if (!file) throw std::runtime_error("NXEmu VFS file not found");
                            vfs.push_back(file);
                        }
                    }
#endif
                    if (phase=="replay") {
                        std::uint64_t rng=seed^(0x9e3779b97f4a7c15ull*(worker+1));
                        std::uint64_t sequential=(file_size/thread_count)*worker;
                        for (std::size_t i=0;i<per_thread;++i) {
                            const auto sample=next_sample(mode,file_size,chunk_size,rng,sequential,i);
                            const auto part_size=file_size/(multi ? 16 : 1);
                            const auto index=static_cast<std::size_t>(sample.offset/part_size);
                            const bool ok=loose_cache ? loose_cache->read(sample.offset,bytes.data(),sample.length) :
                                session ? static_cast<bool>(session->read(file_ids[index],sample.offset%part_size,sample.length,bytes.data(),bytes.size())) :
                                disk->read_at(sample.offset,bytes.data(),sample.length);
                            if (!ok) { result.failed=true; break; }
                        }
                    }
                } catch (...) { result.failed=true; }
                {
                    std::unique_lock<std::mutex> lock(start_mutex);
                    ++ready;
                    start_cv.notify_all();
                    start_cv.wait(lock,[&] { return go; });
                }
                if (result.failed) return;
                try {
                    std::uint64_t rng=seed^(0x9e3779b97f4a7c15ull*(worker+1));
                    std::uint64_t sequential=(file_size/thread_count)*worker;
                    for (std::size_t i=0;i<per_thread;++i) {
                        auto sample=next_sample(mode,file_size,chunk_size,rng,sequential,i);
                        const auto part_size=file_size/(multi ? 16 : 1);
                        const auto index=static_cast<std::size_t>(sample.offset/part_size);
                        const auto started=std::chrono::steady_clock::now();
                        const bool success=loose_cache ? loose_cache->read(sample.offset,bytes.data(),sample.length) :
                            !session ? disk->read_at(sample.offset,bytes.data(),sample.length) :
#if defined(NX_BENCH_NXEMU)
                            vfs[index]->Read(bytes.data(),sample.length,static_cast<std::size_t>(sample.offset%part_size))==sample.length;
#else
                            static_cast<bool>(session->read(file_ids[index],sample.offset%part_size,sample.length,
                                bytes.data(),bytes.size(),mode=="uniform4k" || mode=="hot4k" || mode=="cross4k"
                                    ? std::shared_ptr<nx::ReadCursor>{} : cursor));
#endif
                        const auto ended=std::chrono::steady_clock::now();
                        if (!success) { result.failed=true; break; }
                        sample.latency_us=std::chrono::duration<double,std::micro>(ended-started).count();
                        result.samples.push_back(sample);
                        result.bytes+=sample.length;
                        // Correctness checksum outside the timed read call.
                        update_checksum(result.checksum,bytes.data(),sample.length);
                    }
                } catch (...) { result.failed=true; }
            });
        }
        {
            std::unique_lock<std::mutex> lock(start_mutex);
            start_cv.wait(lock,[&] { return ready==thread_count; });
        }
        auto source_calls=[&] {
#if defined(NX_BENCH_NXEMU)
            if (session) return FileSys::NxSessionStatistics(*session).source_read_calls;
#endif
            if (loose_tree) { std::uint64_t total=0; for (auto& file:loose_tree->files) total+=file->read_calls(); return total; }
            return static_cast<nx::DiskSource*>(disk.get())->read_calls();
        };
        auto source_bytes=[&] {
#if defined(NX_BENCH_NXEMU)
            if (session) return FileSys::NxSessionStatistics(*session).source_read_bytes;
#endif
            if (loose_tree) { std::uint64_t total=0; for (auto& file:loose_tree->files) total+=file->bytes_read(); return total; }
            return static_cast<nx::DiskSource*>(disk.get())->bytes_read();
        };
        const auto read_calls_before=source_calls();
        const auto bytes_before=source_bytes();
        const auto cache_before=session ? session->stats().cache : loose_cache ? loose_cache->stats() : nx::CacheStats{};
        RssSampler rss_sampler(static_cast<unsigned>(rss_interval));
        const auto ready_metrics=process_metrics();
        const auto allocations_before=cpp_allocations.load(),allocated_before=cpp_allocated_bytes.load();
        const auto begin=std::chrono::steady_clock::now();
        {
            std::lock_guard<std::mutex> lock(start_mutex);
            go=true;
        }
        start_cv.notify_all();
        for (auto& t:threads) t.join();
        const auto finish=std::chrono::steady_clock::now();
        rss_sampler.stop();
        const auto finished_metrics=process_metrics();
        const auto allocations=cpp_allocations.load()-allocations_before,allocated=cpp_allocated_bytes.load()-allocated_before;
        for (const auto& result:results)
            if (result.failed || result.samples.size()!=per_thread)
                throw std::runtime_error("checked read failed");
        std::vector<double> latencies;
        latencies.reserve(static_cast<std::size_t>(per_thread*thread_count));
        std::uint64_t bytes=0, checksum=0;
        for (const auto& result:results) {
            bytes+=result.bytes;
            checksum^=result.checksum;
            for (const auto& sample:result.samples) latencies.push_back(sample.latency_us);
        }
        std::sort(latencies.begin(),latencies.end());
        const auto wall=std::chrono::duration<double>(finish-begin).count();
        const auto counters=session ? session->stats() : nx::SessionStats{};
        const auto cache=session ? counters.cache : loose_cache ? loose_cache->stats() : nx::CacheStats{};
        if (sample_path!="-") {
            std::ofstream output(sample_path,std::ios::binary);
            if (!output) throw std::runtime_error("cannot open sample output");
            output<<"worker,index,offset,bytes,latency_us\n";
            for (std::size_t w=0;w<results.size();++w)
                for (std::size_t i=0;i<results[w].samples.size();++i) {
                    const auto& sample=results[w].samples[i];
                    output<<w<<','<<i<<','<<sample.offset<<','<<sample.length<<','
                          <<std::fixed<<std::setprecision(3)<<sample.latency_us<<'\n';
                }
            if (!output) throw std::runtime_error("sample output failed");
        }
        std::cout<<"profile,codec,block_kib,trace,cache_mib,prefetch,threads,samples,"
                    "logical_mib,elapsed_s,p50_us,p95_us,p99_us,p999_us,hits,misses,"
                    "evictions,decodes,physical_read_calls,physical_read_bytes,checksum,"
                    "hash_provider,phase,baseline_rss_bytes,idle_rss_bytes,ready_rss_bytes,"
                    "end_rss_bytes,peak_rss_bytes,cpu_s,cpp_allocations,cpp_allocated_bytes,"
                    "cache_bytes,cache_peak_bytes,metadata_wire_bytes,decode_ns,archive_bytes,metadata_capacity_bytes,"
                    "decoder_bytes,decoder_peak_bytes,decoder_contexts,decoder_pool_bytes,backend,mean_rss_bytes,rss_samples,rss_interval_ms\n";
        std::cout<<profile<<','<<codec<<','<<block_kib<<','<<mode<<','<<cache_mib<<','
                 <<prefetch<<','<<thread_count<<','<<per_thread*thread_count<<','
                 <<std::fixed<<std::setprecision(3)<<(double(bytes)/1048576.0)<<','<<wall<<','
                 <<percentile(latencies,.50)<<','<<percentile(latencies,.95)<<','
                 <<percentile(latencies,.99)<<','<<percentile(latencies,.999)<<','
                 <<cache.hits-cache_before.hits<<','<<cache.misses-cache_before.misses<<','
                 <<cache.evictions-cache_before.evictions<<','<<cache.decompressions-cache_before.decompressions<<','
                 <<source_calls()-read_calls_before<<','
                 <<source_bytes()-bytes_before<<','<<checksum<<','
                 <<(!session ? "not-used" : "crc32c")<<','<<phase<<','
                 <<process_baseline.rss<<','<<idle.rss<<','<<ready_metrics.rss<<','
                 <<finished_metrics.rss<<','<<finished_metrics.peak<<','<<std::setprecision(6)
                 <<finished_metrics.cpu_s-ready_metrics.cpu_s<<','<<allocations<<','<<allocated<<','
                 <<cache.bytes<<','<<cache.peak_bytes<<','<<metadata_wire_bytes<<','
                 <<cache.decompression_ns-cache_before.decompression_ns<<','<<disk->size()<<','
                 <<(session ? session->archive().metadata_capacity_bytes() : 0)<<',';
#if defined(NX_BENCH_NXEMU)
        const auto runtime=session ? FileSys::NxSessionStatistics(*session) : FileSys::NxRuntimeStats{};
        std::cout<<runtime.decoder_bytes<<','<<runtime.decoder_peak_bytes<<','<<runtime.decoder_contexts<<','<<runtime.decoder_pool_bytes<<",nxemu-vfs";
#else
        std::cout<<decoder_bytes.load()<<','<<decoder_peak.load()<<','<<decoder_contexts.load()<<",0,cpp17-session";
#endif
        std::cout<<','<<rss_sampler.mean()<<','<<rss_sampler.samples<<','<<rss_interval<<'\n';
        return 0;
    } catch (const std::exception& error) {
        std::cerr<<"session trace benchmark failed: "<<error.what()<<'\n';
        return 1;
    }
}
