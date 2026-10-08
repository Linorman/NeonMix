// Real Worker state-machine barriers, without a listener or audio device.
#define NEONMIX_AUDIO_TEST_SEAM 1
#define main receiver_main
#include "main.cpp"
#undef main
#include <future>
static void require(bool value,const char *why){if(!value)throw std::runtime_error(why);}
static void command(Worker &w,const std::string &json){auto p=parse(json);w.command(p);plist_free(p);}
int main(){
    try{
        const std::string grant=R"({"type":"grant","worker_generation":7,"connection_id":2,"request_id":3,"session_id":4,"stream_id":5,"stream_epoch":6,"format_epoch":7,"mapping_id":8})";
        const std::string cancel=R"({"type":"disconnect","worker_generation":7,"connection_id":2,"request_id":3,"session_id":4,"stream_epoch":6})";
        auto wait_closed=[](Worker &w){std::unique_lock<std::mutex>l(w.state_mutex);
            require(w.admission_cv.wait_for(l,std::chrono::seconds(2),[&]{return !w.closing&&!w.admitted&&!w.pending_connection;}),"owner close did not converge");};
        for(auto phase:{SessionBarrier::AdmissionPending,SessionBarrier::AdmittedAwaitGrant,SessionBarrier::GrantBeforeInstall,SessionBarrier::GrantInstalled,SessionBarrier::GrantApplied}) {
            running=true;Worker w;w.worker_generation=7;w.admission_id=2;w.known.insert("owner");w.verified_keys[2]="owner";
            std::promise<void> reached,release;auto ready=reached.get_future();auto resume=release.get_future().share();
            w.barrier=[&](SessionBarrier point){
                if(point==SessionBarrier::AdmissionPending && phase!=SessionBarrier::AdmissionPending)
                    command(w,R"({"type":"admit","worker_generation":7,"connection_id":2,"request_id":3,"allowed":true})");
                if(point==phase){reached.set_value();resume.wait();}
            };
            bool allowed=false;
            std::thread establish([&]{Worker::request(&w,2,nullptr,nullptr,nullptr,&allowed);if(allowed)command(w,grant);});
            require(ready.wait_for(std::chrono::seconds(2))==std::future_status::ready,"barrier not reached");
            command(w,cancel);command(w,cancel);
            require(!w.granted,"cancel did not close gate before cleanup");
            release.set_value();establish.join();wait_closed(w);w.barrier={};
            command(w,R"({"type":"admit","worker_generation":7,"connection_id":2,"request_id":3,"allowed":true})");command(w,grant);
            require(!w.admitted&&!w.granted,"late admit/grant revived cancelled owner");
            require(w.cancelled.size()==1&&w.packets.empty()&&!w.pipeline,"cancel leaked state/storage");
            std::cout<<"cancel barrier "<<(int)phase<<" passed\n";
        }
        for(float volume:{-144.f,-18.f,0.f}) {
            running=true;Worker w;w.worker_generation=7;w.known.insert("owner");
            auto admit=[&](uint64_t connection){w.verified_keys[connection]="owner";w.barrier=[&](SessionBarrier point){if(point==SessionBarrier::AdmissionPending)
                command(w,"{\"type\":\"admit\",\"worker_generation\":7,\"connection_id\":"+std::to_string(connection)+",\"request_id\":"+std::to_string(w.admission_id)+",\"allowed\":true}");};
                bool allowed=false;Worker::request(&w,connection,nullptr,nullptr,nullptr,&allowed);require(allowed,"new owner admission failed");w.barrier={};};
            admit(20);Worker::volume(&w,20,volume);float legal=w.gain;
            bool repeated=false;Worker::request(&w,20,nullptr,nullptr,nullptr,&repeated);require(repeated&&w.gain==legal,"repeat SETUP reset volume");
            Worker::flush(&w,20);require(w.gain==legal,"same session FLUSH reset volume");
            Worker::destroy(&w,20);wait_closed(w);admit(21);require(w.gain==1.f,"successor inherited protocol volume");
            Worker::volume(&w,20,-144.f);Worker::destroy(&w,20);require(w.admitted&&w.gain==1.f,"old connection changed successor");
            command(w,"{\"type\":\"disconnect\",\"worker_generation\":6,\"connection_id\":21,\"request_id\":"+std::to_string(w.active_request)+",\"session_id\":4,\"stream_epoch\":6}");
            require(w.admitted,"old worker generation cancelled successor");
            std::cout<<"session volume "<<volume<<" passed\n";
        }
        {running=true;Worker w;w.worker_generation=7;w.admitted=true;w.granted=true;w.active_connection=2;w.active_request=3;w.context={4,5,6,7,8};
            command(w,R"({"type":"disconnect","worker_generation":7,"connection_id":2,"request_id":3,"session_id":0,"stream_epoch":0})");
            require(w.admitted&&w.granted,"active owner accepted zero context");
            command(w,R"({"type":"disconnect","worker_generation":7,"connection_id":2,"request_id":2,"session_id":4,"stream_epoch":6})");
            require(w.admitted&&w.granted,"old request cancelled active successor");command(w,cancel);wait_closed(w);
            require(w.cancelled.size()==1,"invalid cancellation changed tombstones");}
        return 0;
    }catch(const std::exception &e){std::cerr<<e.what()<<'\n';return 1;}
}
