// SPDX-License-Identifier: GPL-3.0-or-later
// Isolated audio receiver: no renderer, audio device, video decoder, or mDNS owner.
#include "platform.h"
#include <gst/gst.h>
#include <gst/app/gstappsrc.h>
#include <gst/app/gstappsink.h>
#include <plist/plist.h>
extern "C" {
#include "raop.h"
#include "logger.h"
}
#include <signal.h>
#include <algorithm>
#include <atomic>
#include <chrono>
#include <cmath>
#include <condition_variable>
#include <cstring>
#include <deque>
#include <iostream>
#include <map>
#include <mutex>
#include <set>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>
#ifndef _WIN32
#include <sys/un.h>
#endif
using Clock = std::chrono::steady_clock;
static std::atomic<bool> running{true};
static std::mutex event_mutex;
static std::string quote(const std::string &s) {
    std::string out="\"";
    for (unsigned char c:s) { if(c=='"'||c=='\\') {out+='\\';out+=c;} else if(c<32) out+=' ';else out+=c; }
    return out+'"';
}
static void event(const std::string &type,const std::string &fields="") {
    std::lock_guard<std::mutex> l(event_mutex);
    std::string line="{\"type\":"+quote(type)+(fields.empty()?"":","+fields)+"}\n";
    if(line.size()>4096||platform::write_event(line.data(),line.size())!=(platform::Count)line.size()) running=false;
}
static void fatal(const std::string &message) {event("fatal","\"message\":"+quote(message));running=false;}
static plist_t parse(const std::string &s) {
    plist_t p=nullptr;
    if(s.size()>16384||plist_from_json(s.data(),s.size(),&p)!=PLIST_ERR_SUCCESS||!PLIST_IS_DICT(p)) {
        if(p)plist_free(p);throw std::runtime_error("invalid control JSON");
    }
    return p;
}
static std::string str(plist_t p,const char *k,const std::string &fallback="") {
    char *v=nullptr;auto n=plist_dict_get_item(p,k);if(!n)return fallback;
    if(!PLIST_IS_STRING(n))throw std::runtime_error(std::string("string required: ")+k);
    plist_get_string_val(n,&v);std::string s=v?v:"";free(v);return s;
}
static uint64_t number(plist_t p,const char*k,uint64_t fallback=0) {
    auto n=plist_dict_get_item(p,k);if(!n)return fallback;
    if(!PLIST_IS_UINT(n))throw std::runtime_error(std::string("integer required: ")+k);
    uint64_t v=0;plist_get_uint_val(n,&v);return v;
}
static bool boolean_field(plist_t p,const char*k,bool fallback=false) {
    auto n=plist_dict_get_item(p,k);if(!n)return fallback;
    if(!PLIST_IS_BOOLEAN(n))throw std::runtime_error(std::string("boolean required: ")+k);
    uint8_t v=0;plist_get_bool_val(n,&v);return v;
}
static std::set<std::string> keyset(plist_t p,const char*k) {
    std::set<std::string> out;auto n=plist_dict_get_item(p,k);if(!n)return out;
    if(!PLIST_IS_ARRAY(n)||plist_array_get_size(n)>64)throw std::runtime_error("invalid client key list");
    for(uint32_t i=0;i<plist_array_get_size(n);i++){char*v=nullptr;auto x=plist_array_get_item(n,i);if(!PLIST_IS_STRING(x))throw std::runtime_error("invalid client key");plist_get_string_val(x,&v);out.emplace(v);free(v);}
    return out;
}
struct Context { uint64_t session=0,stream=0,epoch=0,format=0,mapping=0; };
struct Mark {uint64_t pts,position,uncertainty;};
struct Worker {
    raop_t *raop=nullptr;dnssd_t *dns=nullptr;platform::Socket media=platform::invalid_socket;
#ifdef _WIN32
    platform::MediaPipe media_pipe;
#endif
    std::mutex state_mutex,decode_mutex,queue_mutex;std::condition_variable admission_cv,pairing_cv,queue_cv;
    Context context;std::atomic<bool> admitted{false},granted{true},pairing_enabled{true};
    Clock::time_point trace_started=Clock::now();
    bool protocol_trace_enabled=false;bool admission_answer=false,admission_pending=false;uint64_t admission_id=0,authorization_generation=0;std::set<std::string> known,blocked;
    uint64_t trust_generation=1,pairing_window_generation=1;std::map<uint64_t,uint64_t> pairing_generations,pairing_slots;
    uint64_t pairing_request_sequence=0,pairing_pending_connection=0,pairing_pending_request=0;
    bool pairing_pending=false,pairing_answer=false;
    uint64_t worker_generation=0,active_connection=0,active_request=0,pending_connection=0;
    std::map<uint64_t,std::string> verified_keys;std::string verified_key;Clock::time_point pin_deadline;unsigned pairing_attempts=0;
    GstElement *pipeline=nullptr,*source=nullptr,*sink=nullptr;unsigned codec=0,source_rate=44100,spf=352;
    std::deque<Mark> marks;uint32_t previous_rtp=0;uint64_t extended_rtp=0;bool have_rtp=false;
    uint64_t last_audio_pts=0;uint32_t last_audio_rtp=0;
    std::deque<std::vector<uint8_t>> packets;uint64_t sequence=0;std::atomic<float> gain{1.0f};
    std::thread writer;
    std::string provenance_locked(uint64_t connection=0,uint64_t request=0) const {
        return "\"worker_generation\":"+std::to_string(worker_generation)+",\"connection_id\":"+std::to_string(connection?connection:active_connection)+",\"request_id\":"+std::to_string(request?request:active_request);
    }
    std::string context_fields_locked() const {
        return provenance_locked()+",\"session_id\":"+std::to_string(context.session)+",\"stream_epoch\":"+std::to_string(context.epoch);
    }
    std::string context_fields() {std::lock_guard<std::mutex>l(state_mutex);return context_fields_locked();}
    bool owns(uint64_t connection) {std::lock_guard<std::mutex>l(state_mutex);return admitted && active_connection==connection;}
    void set_context(plist_t p) {std::lock_guard<std::mutex>l(state_mutex);
        context={number(p,"session_id"),number(p,"stream_id"),number(p,"stream_epoch"),number(p,"format_epoch"),number(p,"mapping_id")};
        constexpr uint64_t control_max=(1ULL<<53)-1;
        if(!context.session||!context.stream||!context.epoch||!context.format||!context.mapping ||
           context.session>control_max||context.stream>control_max||context.epoch>control_max||context.format>control_max||context.mapping>control_max)
            throw std::runtime_error("Hub control context must be in 1..2^53-1; refusing possible JSON integer truncation");
    }
    void clear_decoder() {if(pipeline){gst_element_set_state(pipeline,GST_STATE_NULL);gst_object_unref(pipeline);}pipeline=source=sink=nullptr;codec=0;{std::lock_guard<std::mutex>l(state_mutex);marks.clear();have_rtp=false;}}
    // Caller holds decode_mutex. This invalidates only media, keeping the
    // verified source and RTSP owner. Hub supplies a fresh epoch before PCM.
    void reset_media_locked(const char *reason) {
        granted=false;clear_decoder();last_audio_pts=0;
        {std::lock_guard<std::mutex>l(queue_mutex);packets.clear();}
        event("flush",context_fields()+",\"reason\":"+quote(reason));
    }
    ~Worker(){running=false;queue_cv.notify_all();if(media!=platform::invalid_socket)platform::shutdown_socket(media);if(writer.joinable())writer.join();if(raop)raop_destroy(raop);if(dns)dnssd_destroy(dns);clear_decoder();if(media!=platform::invalid_socket)platform::close_socket(media);}
    platform::Count send_media(const void *bytes,size_t length) {
#ifdef _WIN32
        if(media_pipe.connected())return media_pipe.write(bytes,length,running);
#endif
        return platform::send(media,bytes,length);
    }
    void connect_media(const std::string &address,const std::string &token) {
        if(token.size()!=64||token.find_first_not_of("0123456789abcdef")!=std::string::npos)throw std::runtime_error("invalid media IPC credentials");
#ifdef _WIN32
        if(address.rfind("\\\\.\\pipe\\",0)==0){media_pipe.connect(address);}else
#else
        if(address.rfind("unix:",0)==0){
            std::string path=address.substr(5);sockaddr_un a{};struct stat info{};
            if(path.empty()||path[0]!='/'||path.size()>=sizeof(a.sun_path)||path.find('\0')!=std::string::npos||
               lstat(path.c_str(),&info)||!S_ISSOCK(info.st_mode)||info.st_uid!=getuid()||(info.st_mode&0777)!=0600)
                throw std::runtime_error("invalid private media socket");
            media=socket(AF_UNIX,SOCK_STREAM,0);
            if(media==platform::invalid_socket||!platform::configure_media_socket(media))throw std::runtime_error("media IPC socket configuration failed");
            a.sun_family=AF_UNIX;std::memcpy(a.sun_path,path.c_str(),path.size()+1);
            if(connect(media,(sockaddr*)&a,sizeof(a))<0)throw std::runtime_error("media IPC connection failed");
#ifdef __APPLE__
            uid_t peer_uid=0;gid_t peer_gid=0;pid_t peer_pid=0;socklen_t peer_size=sizeof(peer_pid);
            if(getpeereid(media,&peer_uid,&peer_gid)||peer_uid!=getuid()||
               getsockopt(media,SOL_LOCAL,LOCAL_PEERPID,&peer_pid,&peer_size)||peer_size!=sizeof(peer_pid)||peer_pid!=getppid())
                throw std::runtime_error("media IPC Hub identity mismatch");
#endif
        }else
#endif
        {
        auto colon=address.rfind(':');if(colon==std::string::npos||address.substr(0,colon)!="127.0.0.1")throw std::runtime_error("media IPC must be IPv4 loopback");
        int port=std::stoi(address.substr(colon+1));if(port<1||port>65535||token.size()!=64||token.find_first_not_of("0123456789abcdef")!=std::string::npos)throw std::runtime_error("invalid media IPC credentials");
        media=socket(AF_INET,SOCK_STREAM,0);
        if(media==platform::invalid_socket||!platform::configure_media_socket(media))throw std::runtime_error("media IPC socket configuration failed");
        sockaddr_in a{};a.sin_family=AF_INET;a.sin_port=htons(port);inet_pton(AF_INET,"127.0.0.1",&a.sin_addr);
        if(connect(media,(sockaddr*)&a,sizeof(a))<0)throw std::runtime_error("media IPC connection failed");
        }
        std::string auth=token+"\n";if(send_media(auth.data(),auth.size())!=(platform::Count)auth.size())throw std::runtime_error("media IPC authentication write failed");
        writer=std::thread([this]{while(running){std::vector<uint8_t> p;{
            std::unique_lock<std::mutex>l(queue_mutex);queue_cv.wait_for(l,std::chrono::milliseconds(100),[this]{return !packets.empty()||!running;});if(packets.empty())continue;p=std::move(packets.front());packets.pop_front();}
            size_t offset=0;while(offset<p.size()&&running){platform::Count n=send_media(p.data()+offset,p.size()-offset);if(n<=0){if(running)fatal("media IPC stalled or disconnected");break;}offset+=n;}
        }});
    }
    static GstFlowReturn sample(GstAppSink*s,gpointer opaque) {
        auto &w=*(Worker*)opaque;GstSample*sample=gst_app_sink_pull_sample(s);if(!sample)return GST_FLOW_EOS;
        auto buffer=gst_sample_get_buffer(sample);GstMapInfo map{};
        if(!gst_buffer_map(buffer,&map,GST_MAP_READ)){gst_sample_unref(sample);return GST_FLOW_ERROR;}
        uint64_t pts=GST_BUFFER_PTS(buffer);if(pts==GST_CLOCK_TIME_NONE||!w.admitted||!w.granted){gst_buffer_unmap(buffer,&map);gst_sample_unref(sample);return GST_FLOW_OK;}
        Mark mark{pts,0,UINT64_MAX};unsigned rate=44100;Context context;
        {std::lock_guard<std::mutex>l(w.state_mutex);rate=w.source_rate;context=w.context;
            while(w.marks.size()>1 && w.marks[1].pts<=pts)w.marks.pop_front();
            if(w.marks.empty()){gst_buffer_unmap(buffer,&map);gst_sample_unref(sample);return GST_FLOW_OK;}mark=w.marks.front();}
        size_t frames=map.size/8;
        if(map.size%8 || frames>8192){fatal("invalid decoded PCM size");frames=0;}
        for(size_t offset=0;offset<frames;offset+=480){
            size_t count=std::min<size_t>(480,frames-offset);std::vector<uint8_t>p(112+count*8,0);
            auto put=[&p](size_t at,uint64_t v,unsigned n){for(unsigned i=0;i<n;i++)p[at+i]=(uint8_t)(v>>(8*i));};
            std::memcpy(p.data(),"NMAM",4);put(4,1,2);put(6,112,2);put(8,count*8,4);
            put(16,context.session,8);put(24,context.stream,8);put(32,context.epoch,8);put(40,context.format,8);put(48,w.sequence++,8);
            uint64_t chunkpts=pts+(offset*1000000000ULL)/48000;
            int64_t dt=(int64_t)chunkpts-(int64_t)mark.pts;
            int64_t position=(int64_t)mark.position+dt*(int64_t)rate/1000000000LL;
            if(position<0){fatal("invalid source timeline");break;}
            put(56,(uint64_t)position,8);put(64,chunkpts,8);put(72,context.mapping,8);put(80,mark.uncertainty,8);
            put(88,rate,4);put(92,count,2);p[94]=2;p[95]=1;float gain=w.gain;uint32_t bits;memcpy(&bits,&gain,4);put(96,bits,4);p[101]=1;put(104,48000,4);
            std::memcpy(p.data()+112,map.data+offset*8,count*8);
            {std::lock_guard<std::mutex>l(w.queue_mutex);if(w.packets.size()>=200){fatal("PCM queue limit exceeded");break;}w.packets.emplace_back(std::move(p));}w.queue_cv.notify_one();
        }
        gst_buffer_unmap(buffer,&map);gst_sample_unref(sample);return GST_FLOW_OK;
    }
    // Caller holds decode_mutex, so the negotiated format and installed context
    // form one observation even when RTSP SETUP won the race with Hub grant.
    void emit_format() {
        const char *name=codec==1?"pcm_s16":codec==2?"alac":codec==4?"aac_lc":"aac_eld";
        event("format",context_fields()+",\"codec\":"+quote(name)+",\"source_rate\":"+std::to_string(source_rate)+",\"source_frame_count\":"+std::to_string(spf));
    }
    void decoder(unsigned ct,unsigned rate,unsigned frames) {
        clear_decoder();codec=ct;{std::lock_guard<std::mutex>l(state_mutex);source_rate=rate;}spf=frames;
        if(rate!=44100)throw std::runtime_error("unsupported source rate");
        std::string caps,decode;
        if(ct==1)caps="audio/x-raw,format=S16LE,layout=interleaved,channels=2,rate=44100";
        else if(ct==2){caps="audio/x-alac,mpegversion=4,channels=2,rate=44100,stream-format=raw,codec_data=(buffer)00000024616c616300000000";
            unsigned char cookie[24]={};cookie[0]=frames>>24;cookie[1]=frames>>16;cookie[2]=frames>>8;cookie[3]=frames;cookie[5]=16;cookie[6]=40;cookie[7]=10;cookie[8]=14;cookie[9]=2;cookie[10]=0;cookie[11]=255;cookie[20]=rate>>24;cookie[21]=rate>>16;cookie[22]=rate>>8;cookie[23]=rate;
            const char*hex="0123456789abcdef";for(unsigned char c:cookie){caps+=hex[c>>4];caps+=hex[c&15];}decode="avdec_alac ! ";}
        else if(ct==4){caps="audio/mpeg,mpegversion=4,channels=2,rate=44100,stream-format=raw,codec_data=(buffer)1210";decode="avdec_aac ! ";}
        else if(ct==8){caps="audio/mpeg,mpegversion=4,channels=2,rate=44100,stream-format=raw,codec_data=(buffer)f8e85000";decode="avdec_aac ! ";}
        else throw std::runtime_error("unsupported audio codec");
        emit_format();
        std::string launch="appsrc name=source is-live=true format=time block=false max-bytes=262144 ! "+decode+"audioconvert ! audioresample ! audio/x-raw,format=F32LE,rate=48000,channels=2,layout=interleaved ! appsink name=pcm sync=false max-buffers=8 drop=false";
        GError *error=nullptr;pipeline=gst_parse_launch(launch.c_str(),&error);if(error||!pipeline){std::string msg=error?error->message:"pipeline";if(error)g_error_free(error);throw std::runtime_error(msg);}
        source=gst_bin_get_by_name(GST_BIN(pipeline),"source");sink=gst_bin_get_by_name(GST_BIN(pipeline),"pcm");
        auto inputcaps=gst_caps_from_string(caps.c_str());gst_app_src_set_caps(GST_APP_SRC(source),inputcaps);gst_caps_unref(inputcaps);
        GstAppSinkCallbacks callbacks{};callbacks.new_sample=sample;gst_app_sink_set_callbacks(GST_APP_SINK(sink),&callbacks,this,nullptr);
        gst_object_unref(source);gst_object_unref(sink);
        if(gst_element_set_state(pipeline,GST_STATE_PLAYING)==GST_STATE_CHANGE_FAILURE)throw std::runtime_error("audio decoder startup failed");
    }
    static void audio(void*cls,uint64_t connection,raop_ntp_t*ntp,audio_decode_struct *data) {
        auto&w=*(Worker*)cls;
        std::lock_guard<std::mutex>l(w.decode_mutex);if(!w.owns(connection)||!w.granted)return;
        try {
            if(data->data_len<=0||data->data_len>65536||!data->ntp_time_local)return;
            if(w.last_audio_pts && data->source_rate){
                long double pts_delta=(long double)data->ntp_time_local-(long double)w.last_audio_pts;
                long double rtp_delta=(long double)(int32_t)(data->rtp_time-w.last_audio_rtp)*1000000000.L/data->source_rate;
                if(std::fabs(pts_delta-rtp_delta)>50000000.L){w.reset_media_locked("timestamp_jump");return;}
            }
            w.last_audio_pts=data->ntp_time_local;w.last_audio_rtp=data->rtp_time;
            if(!w.pipeline||w.codec!=data->ct)w.decoder(data->ct,data->source_rate,w.spf);
            {std::lock_guard<std::mutex>s(w.state_mutex);
                if(!w.have_rtp){w.extended_rtp=data->rtp_time;w.have_rtp=true;}else w.extended_rtp+=(int32_t)(data->rtp_time-w.previous_rtp);
                w.previous_rtp=data->rtp_time;uint64_t uncertainty=raop_ntp_get_uncertainty_ns(ntp);
#ifdef NEONMIX_AUDIO_TEST_SEAM
                if(!ntp)uncertainty=0;
#endif
                if(uncertainty==UINT64_MAX)return;
                w.marks.push_back({data->ntp_time_local,w.extended_rtp,uncertainty});
                if(w.marks.size()>1024)throw std::runtime_error("decoder metadata limit exceeded");
            }
            if(gst_app_src_get_current_level_bytes(GST_APP_SRC(w.source))>262144)throw std::runtime_error("decoder compressed queue limit exceeded");
            auto b=gst_buffer_new_allocate(nullptr,data->data_len,nullptr);gst_buffer_fill(b,0,data->data,data->data_len);GST_BUFFER_PTS(b)=data->ntp_time_local;GST_BUFFER_DTS(b)=GST_CLOCK_TIME_NONE;GST_BUFFER_DURATION(b)=((uint64_t)w.spf*1000000000ULL)/data->source_rate;
            if(gst_app_src_push_buffer(GST_APP_SRC(w.source),b)!=GST_FLOW_OK)throw std::runtime_error("audio decoder rejected packet");
        }catch(const std::exception&e){fatal(e.what());}
    }
    static void format(void*cls,uint64_t connection,unsigned char*ct,unsigned short*spf,bool*,bool*,uint64_t*) {
        auto&w=*(Worker*)cls;std::lock_guard<std::mutex>l(w.decode_mutex);if(!w.owns(connection))return;
        try{unsigned frames=*spf?*spf:(*ct==2?352:*ct==4?1024:480);if(frames>8192)throw std::runtime_error("invalid source frame size");
            if(w.pipeline && w.admitted)w.reset_media_locked("stream_setup");
            w.decoder(*ct,44100,frames);
        }catch(const std::exception&e){fatal(e.what());}
    }
    static void detail(void*cls,const char*stage,unsigned a_bytes,unsigned proof_bytes,const char*reason){auto&w=*(Worker*)cls;if(w.protocol_trace_enabled)event("protocol_detail","\"stage\":"+quote(stage)+",\"a_bytes\":"+std::to_string(a_bytes)+",\"proof_bytes\":"+std::to_string(proof_bytes)+",\"reason\":"+quote(reason));}
    static void trace(void*cls,const char*method,const char*route,unsigned status){auto&w=*(Worker*)cls;if(w.protocol_trace_enabled)event("protocol","\"method\":"+quote(method)+",\"route\":"+quote(route)+",\"status\":"+std::to_string(status));}
    static void transport(void*cls,const raop_transport_t*m){auto&w=*(Worker*)cls;if(!w.protocol_trace_enabled)return;
        auto elapsed=std::chrono::duration_cast<std::chrono::milliseconds>(Clock::now()-w.trace_started).count();
        event("protocol_transport","\"connection_id\":"+std::to_string(m->connection_id)+",\"event\":"+quote(m->event)+",\"relative_ms\":"+std::to_string(elapsed)+",\"protocol\":"+quote(m->protocol)+",\"method\":"+quote(m->method)+",\"route\":"+quote(m->route)+",\"body_bytes\":"+std::to_string(m->body_bytes)+",\"cseq_present\":"+(m->cseq_present?"true":"false")+",\"session_present\":"+(m->session_present?"true":"false")+",\"status\":"+std::to_string(m->status)+",\"response_bytes\":"+std::to_string(m->response_bytes)+",\"sent_bytes\":"+std::to_string(m->sent_bytes)+",\"outcome\":"+quote(m->outcome));
    }
    std::string pairing_fields_locked(uint64_t connection,uint64_t request,uint64_t trust) const {
        return "\"worker_generation\":"+std::to_string(worker_generation)+",\"trust_generation\":"+std::to_string(trust)+",\"connection_id\":"+std::to_string(connection)+",\"pairing_request_id\":"+std::to_string(request);
    }
    void pairing_end_locked(uint64_t connection) {
        auto slot=pairing_slots.find(connection);if(slot==pairing_slots.end())return;
        event("pairing_ended",pairing_fields_locked(connection,slot->second,pairing_generations[connection]));pairing_slots.erase(slot);
    }
    static bool pairing(void*cls,uint64_t connection,bool new_attempt){auto&w=*(Worker*)cls;std::unique_lock<std::mutex>l(w.state_mutex);
        if(w.admitted||!w.pairing_enabled||Clock::now()>=w.pin_deadline||w.pairing_attempts>5)return false;
        auto attempt=w.pairing_generations.find(connection);
        if(!new_attempt)return attempt==w.pairing_generations.end()||attempt->second==w.trust_generation;
        if(w.pairing_pending)return false;
        w.pairing_end_locked(connection);
        const auto trust=w.trust_generation,request=++w.pairing_request_sequence;
        w.pairing_generations[connection]=trust;w.pairing_slots[connection]=request;
        w.pairing_pending=true;w.pairing_answer=false;w.pairing_pending_connection=connection;w.pairing_pending_request=request;
        event("pairing_request",w.pairing_fields_locked(connection,request,trust));
        bool answered=w.pairing_cv.wait_for(l,std::chrono::milliseconds(500),[&w]{return !w.pairing_pending||!running;});
        bool allowed=answered&&w.pairing_answer&&running&&w.pairing_enabled&&w.trust_generation==trust&&Clock::now()<w.pin_deadline&&w.pairing_attempts<=5;
        w.pairing_pending=false;w.pairing_pending_connection=0;w.pairing_pending_request=0;
        if(!allowed)w.pairing_end_locked(connection);
        return allowed;
    }
    static void pin(void*cls,char*pin){auto&w=*(Worker*)cls;event("pairing_pin","\"worker_generation\":"+std::to_string(w.worker_generation)+",\"pin\":"+quote(pin));}
    static bool check(void*cls,const char*pk){auto&w=*(Worker*)cls;std::lock_guard<std::mutex>l(w.state_mutex);return !w.blocked.count(pk)&&w.known.count(pk);}
    static bool allowed(void*cls,uint64_t connection,const char*pk){auto&w=*(Worker*)cls;std::lock_guard<std::mutex>l(w.state_mutex);
        if(w.admitted && (connection!=w.active_connection||w.verified_key!=pk))return false;
        auto attempt=w.pairing_generations.find(connection);
        bool current_pin=attempt!=w.pairing_generations.end() && attempt->second==w.trust_generation && Clock::now()<w.pin_deadline && w.pairing_attempts<=5;
        return w.pairing_enabled && !w.blocked.count(pk) && (w.known.count(pk) || (current_pin && w.known.size()+w.blocked.size()<64));
    }
    static void verified(void*cls,uint64_t connection,const char*pk){auto&w=*(Worker*)cls;std::lock_guard<std::mutex>l(w.state_mutex);w.verified_keys[connection]=pk?pk:"";}
    static void request(void*cls,uint64_t connection,char*device,char*,char*name,bool*admit){auto&w=*(Worker*)cls;
        std::unique_lock<std::mutex>l(w.state_mutex);
        auto key=w.verified_keys.find(connection);
        if(key==w.verified_keys.end()||key->second.empty()||!w.known.count(key->second)||w.blocked.count(key->second)||!w.pairing_enabled){*admit=false;return;}
        // Repeat SETUP belongs to the existing owner. A second verified socket
        // must never overwrite the active decoder/session or its public key.
        if(w.admitted){*admit=w.active_connection==connection && w.verified_key==key->second;return;}
        if(w.admission_pending){*admit=false;return;}
        w.admission_pending=true;w.admission_answer=false;w.pending_connection=connection;++w.admission_id;
        uint64_t generation=w.authorization_generation,trust=w.trust_generation;std::string verified=key->second;
        event("admit_request","\"trust_generation\":"+std::to_string(trust)+","+w.provenance_locked(connection,w.admission_id)+",\"client_public_key\":"+quote(key->second)+",\"device_id\":"+quote(device?device:"")+",\"name\":"+quote(name?name:""));
        bool answered=w.admission_cv.wait_for(l,std::chrono::milliseconds(500),[&w]{return !w.admission_pending||!running;});
        *admit=answered&&w.admission_answer&&running&&w.pairing_enabled&&generation==w.authorization_generation&&!w.blocked.count(verified);
        w.admission_pending=false;w.pending_connection=0;
        if(*admit){w.admitted=true;w.active_connection=connection;w.active_request=w.admission_id;w.verified_key=verified;w.context={};w.granted=false;
            event("session_started","\"trust_generation\":"+std::to_string(trust)+","+w.context_fields_locked()+",\"client_public_key\":"+quote(w.verified_key)+",\"device_id\":"+quote(device?device:"")+",\"name\":"+quote(name?name:""));}
    }
    static void registered(void*cls,uint64_t connection,const char*device,const char*pk,const char*name){auto&w=*(Worker*)cls;
        std::lock_guard<std::mutex>l(w.state_mutex);
        if(!w.pairing_enabled || Clock::now()>=w.pin_deadline || w.blocked.count(pk))return;
        auto slot=w.pairing_slots.find(connection);if(slot==w.pairing_slots.end())return;
        auto attempt=w.pairing_generations.find(connection);
        if(attempt!=w.pairing_generations.end() && attempt->second!=w.trust_generation)return;
        if(attempt==w.pairing_generations.end() && !w.known.count(pk))return;
        w.known.emplace(pk);
        auto request=connection==w.active_connection?w.active_request:0;
        event("registered","\"pairing_request_id\":"+std::to_string(slot->second)+",\"trust_generation\":"+std::to_string(w.trust_generation)+",\"worker_generation\":"+std::to_string(w.worker_generation)+",\"connection_id\":"+std::to_string(connection)+",\"request_id\":"+std::to_string(request)+",\"client_public_key\":"+quote(pk)+",\"device_id\":"+quote(device?device:"")+",\"name\":"+quote(name?name:""));
        w.pairing_end_locked(connection);
    }
    static void destroy(void*cls,uint64_t connection){auto&w=*(Worker*)cls;
        std::lock_guard<std::mutex>d(w.decode_mutex);
        {std::lock_guard<std::mutex>l(w.state_mutex);w.verified_keys.erase(connection);w.pairing_end_locked(connection);w.pairing_generations.erase(connection);
            if(!w.admitted||connection!=w.active_connection)return;
            w.admitted=false;w.granted=false;event("session_ended",w.context_fields_locked());
            w.active_connection=0;w.active_request=0;w.context={};w.verified_key.clear();}
        w.clear_decoder();{std::lock_guard<std::mutex>q(w.queue_mutex);w.packets.clear();}w.last_audio_pts=0;
    }
    static void flush(void*cls,uint64_t connection){auto&w=*(Worker*)cls;std::lock_guard<std::mutex>l(w.decode_mutex);if(w.owns(connection))w.reset_media_locked("protocol");}
    static double volume_initial(void*){return 0.;}
    static void volume(void*cls,uint64_t connection,float db){auto&w=*(Worker*)cls;std::lock_guard<std::mutex>l(w.state_mutex);if(!std::isfinite(db)||!w.admitted||connection!=w.active_connection)return;w.gain=db<=-144?0.f:std::min(1.f,std::pow(10.f,db/20.f));event("volume",w.context_fields_locked()+",\"volume_db\":"+std::to_string(db));}
    static void reset(void*cls,uint64_t connection,int){flush(cls,connection);}
    void command(plist_t p){auto type=str(p,"type");
        if(type=="stop"){running=false;admission_cv.notify_all();pairing_cv.notify_all();return;}
        if(number(p,"worker_generation")!=worker_generation)return;
        if(type=="pairing_admit"){std::lock_guard<std::mutex>l(state_mutex);
            if(pairing_pending && number(p,"trust_generation")==trust_generation && number(p,"connection_id")==pairing_pending_connection && number(p,"pairing_request_id")==pairing_pending_request){
                auto attempts=number(p,"attempts",UINT64_MAX);if(attempts>6)throw std::runtime_error("invalid pairing attempt count");
                pairing_attempts=std::max(pairing_attempts,(unsigned)attempts);pairing_answer=boolean_field(p,"allowed");pairing_pending=false;
                event("pairing_attempt","\"worker_generation\":"+std::to_string(worker_generation)+",\"attempts\":"+std::to_string(pairing_attempts));pairing_cv.notify_all();}
        }
        else if(type=="pairing_window"){std::lock_guard<std::mutex>l(state_mutex);
            if(number(p,"trust_generation")!=trust_generation||trust_generation<=pairing_window_generation||admitted||admission_pending||pairing_pending)return;
            auto pin=str(p,"pin");auto remaining=number(p,"remaining_ms",UINT64_MAX),attempts=number(p,"attempts",UINT64_MAX);
            if(pin.size()!=4||pin.find_first_not_of("0123456789")!=std::string::npos||remaining>600000||attempts>6)throw std::runtime_error("invalid pairing window");
            while(!pairing_slots.empty())pairing_end_locked(pairing_slots.begin()->first);
            pairing_generations.clear();pairing_window_generation=trust_generation;pin_deadline=Clock::now()+std::chrono::milliseconds(remaining);pairing_attempts=(unsigned)attempts;pairing_enabled=true;
            if(raop)raop_set_plist(raop,"pin",10000+std::stoi(pin));
            event("pairing_window_applied","\"worker_generation\":"+std::to_string(worker_generation)+",\"trust_generation\":"+std::to_string(trust_generation));
        }
        else if(type=="admit"){std::lock_guard<std::mutex>l(state_mutex);
            if(admission_pending && number(p,"connection_id")==pending_connection && number(p,"request_id")==admission_id){admission_answer=boolean_field(p,"allowed");admission_pending=false;admission_cv.notify_all();}}
        else if(type=="grant"){std::lock_guard<std::mutex>d(decode_mutex);
            {std::lock_guard<std::mutex>l(state_mutex);
                if(!admitted||number(p,"connection_id")!=active_connection||number(p,"request_id")!=active_request)return;
                if(context.session && (number(p,"session_id")!=context.session || number(p,"stream_epoch")<=context.epoch))return;
            }
            set_context(p);
            if(pipeline&&codec)emit_format();
            {std::lock_guard<std::mutex>l(state_mutex);event("grant_applied",context_fields_locked()+",\"stream_id\":"+std::to_string(context.stream)+",\"format_epoch\":"+std::to_string(context.format)+",\"mapping_id\":"+std::to_string(context.mapping));}
            granted=true;
        }
        else if(type=="allow"){pairing_enabled=true;}
        else if(type=="trust_update"){
            auto next_known=keyset(p,"known_client_keys"),next_blocked=keyset(p,"blocked_client_keys");
            if(next_known.size()+next_blocked.size()>64)throw std::runtime_error("client key limit exceeded");
            std::lock_guard<std::mutex>l(state_mutex);
            auto next_generation=number(p,"trust_generation");if(next_generation<=trust_generation || next_generation>=(1ULL<<53))return;
            while(!pairing_slots.empty())pairing_end_locked(pairing_slots.begin()->first);
            if(pairing_pending){pairing_pending=false;pairing_answer=false;pairing_cv.notify_all();}
            trust_generation=next_generation;known=std::move(next_known);blocked=std::move(next_blocked);pairing_enabled=boolean_field(p,"pairing_allowed");
            if(admission_pending){++authorization_generation;admission_pending=false;admission_answer=false;admission_cv.notify_all();}
        }
        else if(type=="disconnect"||type=="revoke"){
            uint64_t connection=number(p,"connection_id");
            {std::lock_guard<std::mutex>d(decode_mutex);std::lock_guard<std::mutex>l(state_mutex);
                if(!admitted||connection!=active_connection||number(p,"session_id")!=context.session||number(p,"stream_epoch")!=context.epoch)return;
                ++authorization_generation;granted=false;
                if(type=="revoke"&&!verified_key.empty()){blocked.insert(verified_key);known.erase(verified_key);}
            }
            destroy(this,connection);
            if(raop)raop_remove_connection_id(raop,connection);
        }else throw std::runtime_error("unsupported control command");
    }
};
static void signal_stop(int){running=false;}
int main(){
    signal(SIGINT,signal_stop);signal(SIGTERM,signal_stop);
#ifdef SIGPIPE
    signal(SIGPIPE,SIG_IGN);
#endif
    try {
        platform::Runtime runtime;
        Worker worker;
        platform::configure_control_output();
        std::string first;char ch;bool startup_complete=false;
        auto startup_deadline=Clock::now()+std::chrono::seconds(5);
        while(running && Clock::now()<startup_deadline){
            auto count=platform::read_control(&ch,1,100);
            if(count<0)throw std::runtime_error("startup control disconnected");
            if(!count)continue;
            if(ch=='\n'){startup_complete=true;break;}
            first+=ch;if(first.size()>16384)throw std::runtime_error("startup JSON too large");
        }
        if(!startup_complete)throw std::runtime_error("startup control timed out");
        auto config=parse(first);
        if(number(config,"control_version")!=2)throw std::runtime_error("control IPC v2 required");
        worker.worker_generation=number(config,"worker_generation");
        if(!worker.worker_generation||worker.worker_generation>=(1ULL<<53))throw std::runtime_error("worker_generation must be in 1..2^53-1");
        worker.trust_generation=number(config,"trust_generation");
        if(!worker.trust_generation||worker.trust_generation>=(1ULL<<53))throw std::runtime_error("invalid trust generation");
        worker.pairing_window_generation=worker.trust_generation;
        auto remaining=number(config,"pairing_remaining_ms",UINT64_MAX),attempts=number(config,"pairing_attempts",UINT64_MAX);
        if(remaining>600000||attempts>6)throw std::runtime_error("invalid persisted pairing bounds");
        worker.pin_deadline=Clock::now()+std::chrono::milliseconds(remaining);worker.pairing_attempts=(unsigned)attempts;
        worker.set_context(config);
        auto address=str(config,"media_address"),token=str(config,"ipc_token"),device=str(config,"device_id"),keyfile=str(config,"keyfile"),name=str(config,"name","NeonMix"),pin=str(config,"pin"),receiver_uuid=str(config,"receiver_uuid");
        if(device.size()!=12||device.find_first_not_of("0123456789abcdefABCDEF")!=std::string::npos||name.empty()||name.size()>80||pin.size()!=4||pin.find_first_not_of("0123456789")!=std::string::npos)throw std::runtime_error("invalid receiver configuration");
        if(receiver_uuid.size()!=36 || receiver_uuid[8]!='-' || receiver_uuid[13]!='-' || receiver_uuid[18]!='-' || receiver_uuid[23]!='-' || receiver_uuid.find_first_not_of("0123456789abcdefABCDEF-")!=std::string::npos)throw std::runtime_error("valid stable receiver UUID required");
        keyfile=platform::private_key_path(keyfile);
        worker.known=keyset(config,"known_client_keys");worker.blocked=keyset(config,"blocked_client_keys");if(worker.known.size()+worker.blocked.size()>64)throw std::runtime_error("client key limit exceeded");worker.pairing_enabled=boolean_field(config,"pairing_allowed",true);worker.protocol_trace_enabled=boolean_field(config,"protocol_trace",false);
        auto portvalue=number(config,"rtsp_port",0);if(portvalue>65535)throw std::runtime_error("invalid listening port");plist_free(config);
        if(!getenv("GST_PLUGIN_SYSTEM_PATH_1_0") && platform::set_environment("GST_PLUGIN_SYSTEM_PATH_1_0",NEONMIX_AUDIO_PLUGIN_DIR,1))throw std::runtime_error("plugin environment configuration failed");
        if(platform::set_environment("GST_PLUGIN_PATH_1_0","",1))throw std::runtime_error("plugin environment configuration failed");
        if(!getenv("GST_REGISTRY") && platform::set_environment("GST_REGISTRY",NEONMIX_AUDIO_REGISTRY,1))throw std::runtime_error("registry environment configuration failed");
        gst_init(nullptr,nullptr);
        for(const char*feature:{"appsrc","appsink","audioconvert","audioresample","avdec_alac","avdec_aac"}){auto f=gst_element_factory_find(feature);if(!f)throw std::runtime_error(std::string("required audio plugin missing: ")+feature);gst_object_unref(f);}
        worker.connect_media(address,token);
        raop_callbacks_t callbacks{};callbacks.cls=&worker;callbacks.audio_process=Worker::audio;callbacks.audio_get_format=Worker::format;callbacks.audio_flush=Worker::flush;callbacks.audio_set_volume=Worker::volume;callbacks.audio_set_client_volume=Worker::volume_initial;callbacks.conn_destroy=Worker::destroy;callbacks.conn_reset=Worker::reset;callbacks.report_client_request=Worker::request;callbacks.display_pin=Worker::pin;callbacks.register_client=Worker::registered;callbacks.check_register=Worker::check;callbacks.pairing_detail=Worker::detail;callbacks.protocol_trace=Worker::trace;callbacks.protocol_transport=Worker::transport;callbacks.client_allowed=Worker::allowed;callbacks.verified_client=Worker::verified;callbacks.pairing_allowed=Worker::pairing;
        worker.raop=raop_init(&callbacks);if(!worker.raop)throw std::runtime_error("receiver allocation failed");raop_set_log_level(worker.raop,-1);raop_set_log_callback(worker.raop,[](void*,int,const char*){},nullptr);
        if(raop_init2(worker.raop,0,device.c_str(),keyfile.c_str())<0)throw std::runtime_error("receiver identity initialization failed");
        raop_set_receiver_uuid(worker.raop,receiver_uuid.c_str());
        raop_set_plist(worker.raop,"pin",10000+std::stoi(pin));raop_set_plist(worker.raop,"hls",0);
        unsigned char hw[6];for(unsigned i=0;i<6;i++)hw[i]=std::stoul(device.substr(i*2,2),nullptr,16);
        int error=0;worker.dns=dnssd_init(name.c_str(),name.size(),(char*)hw,6,&error,1);if(!worker.dns)throw std::runtime_error("speaker profile initialization failed");
        dnssd_set_receiver_uuid(worker.dns,receiver_uuid.c_str());
        for(int i=0;i<64;i++)dnssd_set_airplay_features(worker.dns,i,0);
        for(int i:{9,11,12,14,18,19,20,27,30})dnssd_set_airplay_features(worker.dns,i,1);
        raop_set_dnssd(worker.raop,worker.dns);unsigned short port=(unsigned short)portvalue;
        if(raop_start_httpd(worker.raop,&port)<0||!port)throw std::runtime_error("RTSP listen failed");
        event("ready","\"control_version\":2,\"worker_generation\":"+std::to_string(worker.worker_generation)+",\"port\":"+std::to_string(port)+",\"public_key\":"+quote(raop_get_public_key(worker.raop))+",\"features\":"+std::to_string(dnssd_get_airplay_features(worker.dns)));
        std::string line;
        while(running){char bytes[2048];auto count=platform::read_control(bytes,sizeof(bytes),100);
            if(count<0){running=false;break;}
            if(count>0){
                for(platform::Count i=0;i<count;i++){if(bytes[i]=='\n'){auto msg=parse(line);worker.command(msg);plist_free(msg);line.clear();}else{line+=bytes[i];if(line.size()>16384)throw std::runtime_error("control JSON too large");}}
            }
            {std::lock_guard<std::mutex>l(worker.decode_mutex);if(worker.pipeline){auto bus=gst_element_get_bus(worker.pipeline);auto message=gst_bus_pop_filtered(bus,(GstMessageType)(GST_MESSAGE_ERROR));if(message){GError*e=nullptr;char*debug=nullptr;gst_message_parse_error(message,&e,&debug);fatal(e?e->message:"decoder error");if(e)g_error_free(e);g_free(debug);gst_message_unref(message);}gst_object_unref(bus);}}
        }
        raop_stop_httpd(worker.raop);
        return 0;
    }catch(const std::exception&e){fatal(e.what());return 1;}
}
