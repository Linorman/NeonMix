// SPDX-License-Identifier: GPL-3.0-or-later
// Offline decoder/IPC seam verification. No server, Apple client or audio device.
#define NEONMIX_AUDIO_TEST_SEAM 1
#define main receiver_main
#include "main.cpp"
#undef main
static std::vector<std::vector<uint8_t>> encoded_packets(unsigned ct) {
    std::vector<std::vector<uint8_t>> packets;
    GError *error=nullptr;
    std::string encoder=ct==2?"avenc_alac":"avenc_aac";
    auto pipe=gst_parse_launch(("appsrc name=in format=time caps=\"audio/x-raw,format=S16LE,rate=44100,channels=2,layout=interleaved\" ! audioconvert ! "+encoder+" ! appsink name=out sync=false").c_str(),&error);
    if(error||!pipe)throw std::runtime_error("fixture encoder unavailable");
    auto src=gst_bin_get_by_name(GST_BIN(pipe),"in");auto sink=gst_bin_get_by_name(GST_BIN(pipe),"out");gst_element_set_state(pipe,GST_STATE_PLAYING);
    std::vector<int16_t> pcm(16384*2);for(size_t i=0;i<pcm.size()/2;i++)pcm[i*2]=pcm[i*2+1]=(int16_t)(std::sin(i*0.04)*10000);
    auto b=gst_buffer_new_allocate(nullptr,pcm.size()*2,nullptr);gst_buffer_fill(b,0,pcm.data(),pcm.size()*2);GST_BUFFER_PTS(b)=0;GST_BUFFER_DURATION(b)=16384ULL*1000000000/44100;gst_app_src_push_buffer(GST_APP_SRC(src),b);gst_app_src_end_of_stream(GST_APP_SRC(src));
    while(auto sample=gst_app_sink_try_pull_sample(GST_APP_SINK(sink),3*GST_SECOND)){GstMapInfo map{};auto buf=gst_sample_get_buffer(sample);gst_buffer_map(buf,&map,GST_MAP_READ);packets.emplace_back(map.data,map.data+map.size);gst_buffer_unmap(buf,&map);gst_sample_unref(sample);}
    gst_element_set_state(pipe,GST_STATE_NULL);gst_object_unref(src);gst_object_unref(sink);gst_object_unref(pipe);
    if(packets.empty())throw std::runtime_error("fixture encoding failed");return packets;
}
static uint64_t field(const std::vector<uint8_t>&p,size_t offset,size_t bytes){uint64_t v=0;for(size_t i=0;i<bytes;i++)v|=(uint64_t)p[offset+i]<<(8*i);return v;}
int main(){
    setenv("GST_PLUGIN_SYSTEM_PATH_1_0",NEONMIX_AUDIO_PLUGIN_DIR,1);setenv("GST_PLUGIN_PATH_1_0","",1);setenv("GST_REGISTRY",NEONMIX_AUDIO_REGISTRY,1);gst_init(nullptr,nullptr);
    try{
        {
            running=true;Worker w;w.worker_generation=7;w.admitted=true;w.granted=false;
            w.active_connection=2;w.active_request=3;w.verified_key="owner";w.known.insert("owner");
            auto command=[&w](const std::string&s){auto p=parse(s);w.command(p);plist_free(p);};
            auto require=[](bool value,const char*message){if(!value)throw std::runtime_error(message);};
            command(R"({"type":"grant","worker_generation":6,"connection_id":2,"request_id":3,"session_id":4,"stream_id":5,"stream_epoch":6,"format_epoch":7,"mapping_id":8})");
            require(!w.granted,"stale process grant applied");
            command(R"({"type":"grant","worker_generation":7,"connection_id":1,"request_id":3,"session_id":4,"stream_id":5,"stream_epoch":6,"format_epoch":7,"mapping_id":8})");
            require(!w.granted,"stale connection grant applied");
            command(R"({"type":"grant","worker_generation":7,"connection_id":2,"request_id":1,"session_id":4,"stream_id":5,"stream_epoch":6,"format_epoch":7,"mapping_id":8})");
            require(!w.granted,"stale request grant applied");
            command(R"({"type":"grant","worker_generation":7,"connection_id":2,"request_id":3,"session_id":4,"stream_id":5,"stream_epoch":6,"format_epoch":7,"mapping_id":8})");
            require(w.granted&&w.context.session==4,"owner grant not applied");
            Worker::destroy(&w,1);Worker::flush(&w,1);Worker::volume(&w,1,-144.f);
            require(w.admitted&&w.granted&&w.gain==1.f,"old connection changed owner");
            command(R"({"type":"disconnect","worker_generation":7,"connection_id":2,"request_id":3,"session_id":4,"stream_epoch":5})");
            require(w.admitted,"stale epoch disconnected owner");
            Worker::flush(&w,2);require(!w.granted&&w.admitted,"owner flush lost session");
            command(R"({"type":"grant","worker_generation":7,"connection_id":2,"request_id":3,"session_id":4,"stream_id":5,"stream_epoch":6,"format_epoch":7,"mapping_id":8})");
            require(!w.granted,"stale epoch reopened media");
            command(R"({"type":"grant","worker_generation":7,"connection_id":2,"request_id":3,"session_id":4,"stream_id":5,"stream_epoch":7,"format_epoch":7,"mapping_id":9})");
            require(w.granted&&w.context.mapping==9,"new epoch grant not applied");
            command(R"({"type":"revoke","worker_generation":7,"connection_id":2,"request_id":3,"session_id":4,"stream_epoch":7})");
            {std::unique_lock<std::mutex>l(w.state_mutex);require(w.admission_cv.wait_for(l,std::chrono::seconds(2),[&]{return !w.closing&&!w.admitted;}),"revoke cleanup timeout");}
            require(!w.admitted&&w.blocked.count("owner"),"targeted revoke failed");
            w.admitted=true;w.active_connection=9;w.active_request=10;w.context={11,12,13,14,15};
            Worker::destroy(&w,2);require(w.admitted,"delayed old owner close ended successor");
            w.admitted=false;w.active_connection=0;w.pin_deadline=Clock::now()+std::chrono::seconds(10);
            bool pairing_result=false;
            std::thread challenger([&]{pairing_result=Worker::pairing(&w,20,true);});
            for(unsigned i=0;i<100;i++){bool pending=false;{std::lock_guard<std::mutex>l(w.state_mutex);pending=w.pairing_pending;}if(pending)break;std::this_thread::sleep_for(std::chrono::milliseconds(1));}
            command(R"({"type":"pairing_admit","worker_generation":7,"trust_generation":1,"connection_id":20,"pairing_request_id":1,"allowed":true,"attempts":1})");
            challenger.join();require(pairing_result,"Hub-authorized pairing challenge rejected");
            w.admission_pending=true;w.admission_answer=true;w.pending_connection=20;w.admission_id=1;
            command(R"({"type":"trust_update","worker_generation":7,"trust_generation":2,"known_client_keys":[],"blocked_client_keys":[],"pairing_allowed":true})");
            require(w.trust_generation==2&&!w.admission_pending&&!w.admission_answer,"trust update retained old admission");
            {std::unique_lock<std::mutex>l(w.state_mutex);require(w.admission_cv.wait_for(l,std::chrono::seconds(2),[&]{return !w.closing;}),"trust cleanup timeout");}
            Worker::registered(&w,20,"device","delayed-key","name");
            require(!w.known.count("delayed-key"),"late registration restored old trust");
            require(!Worker::pairing(&w,20,false),"old SRP phase survived trust change");
            command(R"({"type":"trust_update","worker_generation":7,"trust_generation":1,"known_client_keys":["stale"],"blocked_client_keys":[],"pairing_allowed":true})");
            require(!w.known.count("stale")&&w.trust_generation==2,"stale trust snapshot applied");
            w.pairing_attempts=5;
            require(!Worker::pairing(&w,21,true)&&w.pairing_attempts==5,"unanswered challenge changed Hub attempt budget");
            require(!Worker::allowed(&w,20,"delayed-key"),"removed key retained PIN authority");
            command(R"({"type":"pairing_window","worker_generation":7,"trust_generation":2,"pin":"2468","remaining_ms":600000,"attempts":0})");
            require(w.pairing_attempts==0&&w.pairing_window_generation==2,"new idle pairing window did not apply");
            w.pairing_attempts=4;
            command(R"({"type":"pairing_window","worker_generation":7,"trust_generation":2,"pin":"2468","remaining_ms":600000,"attempts":0})");
            require(w.pairing_attempts==4,"replayed pairing window reset attempt budget");
            bool cancelled_pairing=true;
            std::thread pending_challenge([&]{cancelled_pairing=Worker::pairing(&w,30,true);});
            for(unsigned i=0;i<100;i++){bool pending=false;{std::lock_guard<std::mutex>l(w.state_mutex);pending=w.pairing_pending;}if(pending)break;std::this_thread::sleep_for(std::chrono::milliseconds(1));}
            command(R"({"type":"trust_update","worker_generation":7,"trust_generation":3,"known_client_keys":[],"blocked_client_keys":[],"pairing_allowed":true})");
            pending_challenge.join();require(!cancelled_pairing&&w.pairing_slots.empty(),"trust update did not cancel pending challenge");
            command(R"({"type":"pairing_admit","worker_generation":7,"trust_generation":2,"connection_id":30,"pairing_request_id":3,"allowed":true,"attempts":5})");
            require(w.pairing_attempts==4&&w.pairing_slots.empty(),"late pairing approval revived old challenge");
            std::cout<<"control v2: generation, connection, request, session and epoch fencing passed\n";
        }
        for(unsigned ct:{1,2,4}){
            running=true;Worker w;w.admitted=true;w.active_connection=1;w.context={1,2,3,4,5};w.spf=ct==2?4096:ct==4?1024:352;
            std::vector<std::vector<uint8_t>> packets;if(ct==1)for(unsigned i=0;i<24;i++)packets.emplace_back(w.spf*4,0);else packets=encoded_packets(ct);
            uint64_t start=1780000000000000000ULL;uint32_t rtp=0xffffff00;unsigned index=0;
            for(auto &p:packets){audio_decode_struct d{};d.data=p.data();d.data_len=p.size();d.ct=ct;d.source_rate=44100;d.rtp_time=rtp;d.ntp_time_local=start+(uint64_t)index*w.spf*1000000000/44100+(ct==1?index*4000ULL:0);Worker::audio(&w,1,nullptr,&d);rtp+=w.spf;index++;}
            for(unsigned i=0;i<100;i++){bool ready=false;{std::lock_guard<std::mutex>l(w.queue_mutex);ready=!w.packets.empty();}if(ready)break;std::this_thread::sleep_for(std::chrono::milliseconds(10));}
            {std::lock_guard<std::mutex>l(w.queue_mutex);if(w.packets.empty())throw std::runtime_error("decoder emitted no PCM for ct="+std::to_string(ct));
                uint64_t normalized_end=0;bool first=true;
                for(auto&p:w.packets){if(memcmp(p.data(),"NMAM",4)||field(p,4,2)!=2||field(p,6,2)!=120||field(p,88,4)!=44100||field(p,104,4)!=48000||field(p,92,2)>480||field(p,64,8)<start||field(p,56,8)<0xffffff00)throw std::runtime_error("PCM metadata validation failed");uint64_t normalized=field(p,112,8);if(!first&&normalized!=normalized_end)throw std::runtime_error("SRC chunk coordinate drift: expected="+std::to_string(normalized_end)+" actual="+std::to_string(normalized)+" pts="+std::to_string(field(p,64,8))+" source="+std::to_string(field(p,56,8)));first=false;normalized_end=normalized+field(p,92,2);for(size_t at=120;at<p.size();at+=4){float v;memcpy(&v,p.data()+at,4);if(!std::isfinite(v))throw std::runtime_error("PCM contains invalid values");}}
                std::cout<<"codec "<<ct<<": "<<w.packets.size()<<" bounded PCM chunks; source44100, pcm48000, timestamp and RTP wrap preserved\n";
            }
            w.admitted=false;w.clear_decoder();
        }
        {
            auto require=[](bool condition,const std::string &message){if(!condition)throw std::runtime_error(message);};
            running=true;Worker w;w.admitted=true;w.active_connection=1;w.context={1,2,3,4,5};w.spf=352;
            const uint64_t start=1780000000000000000ULL;uint32_t rtp=0xffffff00;std::vector<uint8_t>pcm(352*4,0);
            for(unsigned i=0;i<24;i++){
                if(i==12)rtp+=441;
                audio_decode_struct d{};d.data=pcm.data();d.data_len=pcm.size();d.ct=1;d.source_rate=44100;d.rtp_time=rtp;
                d.ntp_time_local=start+(uint64_t)(i*352+(i>=12?441:0))*1000000000/44100;
                Worker::audio(&w,1,nullptr,&d);rtp+=352;
            }
            {std::unique_lock<std::mutex>l(w.queue_mutex);require(w.queue_cv.wait_for(l,std::chrono::seconds(2),[&]{return w.packets.size()>=24;}),"gap decoder timeout");
                uint64_t end=0,gap=0;bool first=true;unsigned boundaries=0;
                for(const auto &p:w.packets){uint64_t position=field(p,112,8);if(!first){require(position>=end,"SRC overlap at gap");gap+=position-end;}
                    first=false;end=position+field(p,92,2);boundaries+=field(p,12,4)!=0;}
                require(gap>=479&&gap<=481,"ten-ms source gap was compressed: "+std::to_string(gap));
                require(boundaries==2,"SRC did not isolate gap FIR");
                std::cout<<"source gap: "<<gap<<" internal frames, two SRC/FIR segments preserved\n";
            }
            w.admitted=false;w.clear_decoder();
        }
        return 0;
    }catch(const std::exception&e){std::cerr<<e.what()<<'\n';return 1;}
}
