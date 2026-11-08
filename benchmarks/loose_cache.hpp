#pragma once
#include <list>
#include <map>
#include <mutex>
#include <array>
// Benchmark comparator only: identical block granularity, payload/entry limits,
// range atomicity, LRU copy-under-lock and striped miss coalescing as NX. No
// archive parsing, SHA verification or speculative reads for loose files.
class LooseCache {
    struct Entry { std::vector<std::uint8_t> data; std::list<std::uint64_t>::iterator lru; };
    std::shared_ptr<nx::RandomAccessSource> source_;
    std::uint64_t block_,budget_,resident_=0;
    std::mutex mutex_;
    std::array<std::mutex,32> stripes_;
    std::map<std::uint64_t,Entry> entries_;
    std::list<std::uint64_t> lru_;
    nx::CacheStats stats_;
public:
    LooseCache(std::shared_ptr<nx::RandomAccessSource> source,std::uint64_t block,std::uint64_t budget)
        :source_(std::move(source)),block_(block),budget_(budget) {}
    bool read(std::uint64_t offset,std::uint8_t* output,std::size_t length) {
        if (offset>source_->size() || length>source_->size()-offset) return false;
        if (!length) return true;
        const bool staged=offset/block_!=(offset+length-1)/block_;
        std::vector<std::uint8_t> result(staged ? length : 0);
        auto* destination=staged ? result.data() : output;
        std::size_t done=0;
        while (done<length) {
            const auto at=offset+done, id=at/block_, begin=id*block_;
            const auto logical=static_cast<std::size_t>(std::min(block_,source_->size()-begin));
            const auto within=static_cast<std::size_t>(at-begin);
            const auto count=std::min(length-done,logical-within);
            auto cached=[&] {
                std::lock_guard<std::mutex> lock(mutex_);
                auto found=entries_.find(id);
                if (found==entries_.end()) return false;
                ++stats_.hits;
                lru_.splice(lru_.end(),lru_,found->second.lru);
                std::memcpy(destination+done,found->second.data.data()+within,count);
                return true;
            };
            if (!cached()) {
                std::lock_guard<std::mutex> stripe(stripes_[id%stripes_.size()]);
                if (!cached()) {
                    { std::lock_guard<std::mutex> lock(mutex_); ++stats_.misses; }
                    std::vector<std::uint8_t> data(logical);
                    if (!source_->read_at(begin,data.data(),data.size())) return false;
                    std::lock_guard<std::mutex> lock(mutex_);
                    if (data.size()<=budget_) {
                        while (!lru_.empty() && (resident_+data.size()>budget_ || entries_.size()>=1024)) {
                            auto victim=entries_.find(lru_.front()); resident_-=victim->second.data.size();
                            entries_.erase(victim); lru_.pop_front(); ++stats_.evictions;
                        }
                        const auto bytes=data.size();
                        lru_.push_back(id);
                        decltype(entries_)::iterator entry;
                        try { entry=entries_.emplace(id,Entry{std::move(data),std::prev(lru_.end())}).first; }
                        catch (...) { lru_.pop_back(); throw; }
                        resident_+=bytes;
                        stats_.peak_bytes=std::max(stats_.peak_bytes,resident_);
                        std::memcpy(destination+done,entry->second.data.data()+within,count);
                    } else std::memcpy(destination+done,data.data()+within,count);
                }
            }
            done+=count;
        }
        if (staged) std::memcpy(output,result.data(),length);
        return true;
    }
    nx::CacheStats stats() {
        std::lock_guard<std::mutex> lock(mutex_); auto result=stats_;
        result.bytes=resident_; result.entries=entries_.size(); return result;
    }
};
