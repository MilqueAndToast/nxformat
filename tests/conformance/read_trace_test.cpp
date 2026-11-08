#include "nx_read_trace.hpp"
#include <cassert>
#include <fstream>
#include <iostream>
#include <vector>
int main(int argc,char** argv) {
    assert(argc==2);const std::filesystem::path root(argv[1]);std::filesystem::create_directories(root);
    const auto path=root/"concurrent.tsv";
    auto trace=nx::ReadTrace::open(path);assert(trace);
    const std::string header="1\t0100000000010000\t0100000000010000\t0\t0\t"+std::string(32,'0')+"\t"+std::string(64,'0')+"\t"+std::string(64,'1')+"\t1048576\t"+std::string(64,'2')+"\t66696c65";
    const auto file=trace->file(header);assert(file==1 && trace->file(header)==file);
    std::vector<std::thread> workers;
    for(unsigned t=0;t<4;++t) workers.emplace_back([&]{const auto handle=trace->handle();for(unsigned i=0;i<500;++i) {auto ticket=trace->begin();trace->end(ticket,file,handle,i*512,512);}});
    for(auto& worker:workers) worker.join();trace->close();
    const auto stats=trace->stats();assert(stats.accepted==2000 && stats.dropped==0 && stats.queue_peak<=4096 && !stats.io_failed);
    assert(!nx::ReadTrace::open(path));
    {auto limited=nx::ReadTrace::open(root/"limited.tsv",2);assert(limited);auto f=limited->file(header);auto h=limited->handle();for(unsigned i=0;i<3;++i)limited->end(limited->begin(),f,h,0,4);limited->close();assert(limited->stats().accepted==2 && limited->stats().dropped==1);}
    {auto timed=nx::ReadTrace::open(root/"timed.tsv",100000,1);assert(timed);const auto f=timed->file(header);timed->end(timed->begin(),f,timed->handle(),0,4);
     bool finalized=false;const auto deadline=std::chrono::steady_clock::now()+std::chrono::seconds(5);
     do {std::this_thread::sleep_for(std::chrono::milliseconds(25));std::ifstream input(root/"timed.tsv");std::string text((std::istreambuf_iterator<char>(input)),{});finalized=text.find("Z\t1\t0\n")!=std::string::npos;} while(!finalized && std::chrono::steady_clock::now()<deadline);
     assert(finalized);timed->close();}
    std::cout<<"PASS capture: concurrency, bounded queue/event count, loss signaling, existing-path refusal, timed finalization\n";
}
