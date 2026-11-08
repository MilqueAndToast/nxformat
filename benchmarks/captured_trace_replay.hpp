// Held-out native VFS trace mode in the existing session benchmark.
#pragma once
#include <map>
#include <sstream>
#include "nx_read_trace.hpp"

inline std::vector<std::string> trace_fields(const std::string& row) {
    std::vector<std::string> out;std::stringstream s(row);std::string part;
    while(std::getline(s,part,'\t')) out.push_back(part);
    if(!out.empty() && !out.back().empty() && out.back().back()=='\r') out.back().pop_back();
    return out;
}
inline std::string trace_unhex(const std::string& value) {
    if(value.size()%2) throw std::runtime_error("odd trace hex");
    std::string out;for(size_t i=0;i<value.size();i+=2) out.push_back(static_cast<char>(std::stoul(value.substr(i,2),nullptr,16)));return out;
}
inline int replay_captured(int argc,char** argv) {
    // --captured NX TRACE EXPECTED.tsv RESULT.json [native|nx] [cold|replay]
    if(argc!=8) throw std::runtime_error("usage: --captured NX TRACE EXPECTED.tsv RESULT.json native|nx cold|replay");
    const bool native=std::string(argv[6])=="native";
    const bool warm=std::string(argv[7])=="replay";
    if((!native && std::string(argv[6])!="nx") || (!warm && std::string(argv[7])!="cold")) throw std::runtime_error("bad replay mode");
    nx::SessionOptions settings;settings.reader.block_sha256=&nx::native_block_sha256;
    settings.reader.zstd=&decode;
    settings.sequential_prefetch=false;
#if defined(_WIN32)
    const bool timing=GetEnvironmentVariableA("NX_BENCH_TIMING",nullptr,0)!=0;
#else
    const bool timing=std::getenv("NX_BENCH_TIMING")!=nullptr;
#endif
#if !defined(NX_BENCH_LEGACY_READER)
    settings.reader.collect_read_timing=timing;
#endif
#if defined(NX_BENCH_NXEMU)
    auto opened=FileSys::OpenNxSession(std::make_shared<HostPlaceholder>(),argv[2],settings);
#else
    auto source=nx::DiskSource::open(argv[2]);if(!source) throw std::runtime_error("open replay NX");
    auto opened=nx::Session::open(*source.value,settings);
#endif
    if(!opened) throw std::runtime_error("mount replay NX: "+opened.error.message);
    auto session=*opened.value;
    struct Target {uint32_t snapshot=0;nx::FileId file;uint64_t size=0,offset=0;std::string path;std::shared_ptr<nx::DiskSource> source;
#if defined(NX_BENCH_NXEMU)
        FileSys::VirtualFile vfs;
#endif
    };
    std::map<uint64_t,Target> targets;
    struct Read {uint64_t seq,id,offset;size_t bytes;};std::vector<Read> reads;
    std::ifstream trace(argv[3]);if(!trace) throw std::runtime_error("open trace");
    std::string line;bool header=false,footer=false;size_t total=0,largest=0;
    while(std::getline(trace,line)) {
        total+=line.size()+1;if(total>64*1024*1024 || line.size()>16384 || footer) throw std::runtime_error("trace limits/trailing data");
        auto f=trace_fields(line);
        if(f.size()==1 && f[0]=="NXREAD1" && !header) {header=true;continue;}
        if(!header) throw std::runtime_error("missing trace header");
        if(f.size()==13 && f[0]=="F") {
            uint32_t snapshot=0xffffffffu;
            for(uint32_t i=0;i<session->archive().snapshots().size();++i) {
                const auto& s=session->archive().snapshots()[i];
                if(nx::ReadTrace::hex(s.identity)==f[9]) { if(snapshot!=0xffffffffu) throw std::runtime_error("ambiguous scope");snapshot=i; }
            }
            if(snapshot==0xffffffffu) throw std::runtime_error("captured snapshot absent");
            const auto& s=session->archive().snapshots()[snapshot];
            const nx::Application* app=nullptr;
            for(const auto& a:session->archive().applications()) if(snapshot>=a.first_snapshot && snapshot-a.first_snapshot<a.snapshot_count) app=&a;
            std::ostringstream app_id,program_id;
            if(app) app_id<<std::hex<<std::setw(16)<<std::setfill('0')<<app->id;
            if(s.program_index==0xffffffffu) program_id<<'-'; else program_id<<std::hex<<std::setw(16)<<std::setfill('0')<<session->archive().programs()[s.program_index].id;
            if(!app || std::to_string(app->platform)!=f[2] || app_id.str()!=f[3] || program_id.str()!=f[4] ||
               std::to_string(s.type)!=f[5] || std::to_string(s.numeric_version)!=f[6] || nx::ReadTrace::hex(s.content_id)!=f[7] || nx::ReadTrace::hex(s.build_id)!=f[8]) throw std::runtime_error("captured version/build scope mismatch");
            Target t;t.snapshot=snapshot;t.path=trace_unhex(f[12]);t.size=std::stoull(f[10]);
            auto info=session->archive().resolve_file(snapshot,t.path);if(!info || info.value->size!=t.size) throw std::runtime_error("captured file absent/size mismatch");
            t.file=info.value->id;auto digest=session->archive().file_digest(t.file);
            if(!digest || nx::ReadTrace::hex(*digest.value)!=f[11]) throw std::runtime_error("captured file digest mismatch");
#if defined(NX_BENCH_NXEMU)
            if(!native) {auto dir=FileSys::NxSnapshotDirectory(session,snapshot);std::stringstream parts(t.path);std::string part;while(std::getline(parts,part,'/')) {if(parts.peek()==EOF) t.vfs=dir->GetFile(part);else dir=dir->GetSubdirectory(part);if(!dir) break;}if(!t.vfs) throw std::runtime_error("VFS path unresolved");}
#endif
            if(!targets.emplace(std::stoull(f[1]),std::move(t)).second || targets.size()>4096) throw std::runtime_error("duplicate/excessive trace file");
        } else if(f.size()==9 && f[0]=="R") {
            const auto length=std::stoull(f[6]);if(!length || length>UINT32_MAX || reads.size()==100000) throw std::runtime_error("replay read bounds");
            Read r{std::stoull(f[1]),std::stoull(f[2]),std::stoull(f[5]),static_cast<size_t>(length)};
            auto t=targets.find(r.id);if(t==targets.end() || r.offset>t->second.size || length>t->second.size-r.offset) throw std::runtime_error("replay read extent");
            reads.push_back(r);largest=std::max(largest,std::min<size_t>(r.bytes,4*1024*1024));
        } else if(f.size()==3 && f[0]=="Z") {if(std::stoull(f[1])!=reads.size() || std::stoull(f[2])!=0) throw std::runtime_error("lossy trace");footer=true;}
        else throw std::runtime_error("bad trace record");
    }
    if(!footer || reads.empty()) throw std::runtime_error("incomplete trace");
    std::sort(reads.begin(),reads.end(),[](const auto& a,const auto& b){return a.seq<b.seq;});
    for(size_t i=1;i<reads.size();++i) if(reads[i-1].seq==reads[i].seq) throw std::runtime_error("duplicate trace read order");
    std::map<std::string,std::shared_ptr<nx::DiskSource>> sources;
    if(native) {
        std::ifstream expected(argv[4]);if(!expected) throw std::runtime_error("open native expected manifest");
        std::map<std::pair<uint32_t,std::string>,std::vector<std::string>> locations;
        while(std::getline(expected,line)) {auto f=trace_fields(line);if(f.size()!=5) throw std::runtime_error("native manifest fields");locations.emplace(std::make_pair(static_cast<uint32_t>(std::stoul(f[0])),f[1]),std::move(f));}
        for(auto& pair:targets) {auto& t=pair.second;const auto it=locations.find({t.snapshot,t.path});if(it==locations.end() || std::stoull(it->second[4])!=t.size) throw std::runtime_error("native content location absent");const auto& path=it->second[2];auto found=sources.find(path);if(found==sources.end()) {auto disk=nx::DiskSource::open(path);if(!disk) throw std::runtime_error("open native source");found=sources.emplace(path,*disk.value).first;}t.source=found->second;t.offset=std::stoull(it->second[3]);}
    }
    std::vector<uint8_t> bytes(largest);std::vector<double> latencies;latencies.reserve(reads.size());
    std::array<std::vector<double>,5> classes;
    std::array<uint64_t,5> class_bytes{};
    auto run=[&](bool measure) {
        nx::Sha256 checksum;uint64_t total_bytes=0;
        for(const auto& r:reads) {auto& t=targets.at(r.id);double latency=0;
          for(size_t done=0;done<r.bytes;) {const auto part=std::min(bytes.size(),r.bytes-done);const auto begin=std::chrono::steady_clock::now();bool ok;
            if(native) ok=t.source->read_at(t.offset+r.offset+done,bytes.data(),part);
            else {
#if defined(NX_BENCH_NXEMU)
                ok=t.vfs->Read(bytes.data(),part,r.offset+done)==part;
#else
                ok=bool(session->read(t.file,r.offset+done,part,bytes.data(),bytes.size()));
#endif
            }
            const auto elapsed=std::chrono::duration<double,std::micro>(std::chrono::steady_clock::now()-begin).count();
            if(!ok) {throw std::runtime_error("replay read failed");}
            latency+=elapsed;checksum.update(bytes.data(),part);total_bytes+=part;done+=part;
          }
          if(measure) {latencies.push_back(latency);const auto c=r.bytes<=4096?0:r.bytes<=65536?1:r.bytes<=1048576?2:r.bytes<=4194304?3:4;classes[c].push_back(latency);class_bytes[c]+=r.bytes;}
        }
        return std::make_pair(nx::ReadTrace::hex(checksum.finish()),total_bytes);
    };
    if(warm) run(false);
    cpp_time_allocations=timing;
    const auto allocation_ns_before=cpp_allocation_ns.load();
    const auto alloc_before=cpp_allocations.load(),allocated_before=cpp_allocated_bytes.load();
    const auto before=session->stats().cache;const auto cpu=process_metrics();RssSampler rss(5);const auto begin=std::chrono::steady_clock::now();
    const auto result=run(true);const auto elapsed=std::chrono::duration<double>(std::chrono::steady_clock::now()-begin).count();rss.stop();const auto after=process_metrics();const auto cache=session->stats().cache;
    const auto allocations=cpp_allocations.load()-alloc_before, allocated=cpp_allocated_bytes.load()-allocated_before;
    cpp_time_allocations=false;
    std::sort(latencies.begin(),latencies.end());
    std::ofstream out(argv[5]);out<<std::setprecision(12)<<"{\"mode\":\""<<(native?"native":"nx")<<"\",\"phase\":\""<<(warm?"replay":"cold")<<"\",\"reads\":"<<reads.size()<<",\"bytes\":"<<result.second<<",\"sha256\":\""<<result.first<<"\",\"elapsed_s\":"<<elapsed<<",\"cpu_s\":"<<after.cpu_s-cpu.cpu_s<<",\"rss_peak_bytes\":"<<after.peak<<",\"rss_mean_bytes\":"<<rss.mean()<<",\"p50_us\":"<<percentile(latencies,.5)<<",\"p95_us\":"<<percentile(latencies,.95)<<",\"p99_us\":"<<percentile(latencies,.99)<<",\"p999_us\":"<<percentile(latencies,.999)<<",\"cache_hits\":"<<cache.hits-before.hits<<",\"cache_misses\":"<<cache.misses-before.misses<<",\"decompression_ns\":"<<cache.decompression_ns-before.decompression_ns<<",\"cache_peak_bytes\":"<<cache.peak_bytes<<",\"tail_warning\":\""<<(reads.size()<10000?"fewer than ten p99.9 tail samples":"")<<"\",\"scope\":\"serial replay in request-start order; OS cache uncontrolled; checksum outside per-read latency, included in CPU/throughput\"";
    out<<",\"allocations\":"<<allocations<<",\"allocated_bytes\":"<<allocated
         <<",\"allocation_ns\":"<<cpp_allocation_ns.load()-allocation_ns_before
#if !defined(NX_BENCH_LEGACY_READER)
         <<",\"io_ns\":"<<cache.io_ns-before.io_ns<<",\"hash_ns\":"<<cache.hash_ns-before.hash_ns<<",\"copy_ns\":"<<cache.copy_ns-before.copy_ns
#else
         <<",\"io_ns\":0,\"hash_ns\":0,\"copy_ns\":0"
#endif
         <<",\"evictions\":"<<cache.evictions-before.evictions<<",\"stored_bytes_read\":"<<cache.stored_bytes_read-before.stored_bytes_read<<",\"classes\":[";
    for(size_t i=0;i<classes.size();++i) {auto& c=classes[i];std::sort(c.begin(),c.end());if(i) out<<',';
        out<<"{\"count\":"<<c.size()<<",\"bytes\":"<<class_bytes[i];if(!c.empty()) out<<",\"p50_us\":"<<percentile(c,.5)<<",\"p99_us\":"<<percentile(c,.99)<<",\"p999_us\":"<<percentile(c,.999);out<<'}';}
    out<<"]}\n";
    if(!out) {throw std::runtime_error("write replay output");}
    return 0;
}
