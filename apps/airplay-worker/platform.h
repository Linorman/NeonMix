// SPDX-License-Identifier: GPL-3.0-or-later
// Only local process IPC and private-file checks belong in this adapter.
#pragma once
#include <algorithm>
#include <array>
#include <filesystem>
#include <memory>
#include <atomic>
#include <chrono>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>
#ifdef _WIN32
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#include <aclapi.h>
#include <tlhelp32.h>
#else
#include <arpa/inet.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <poll.h>
#include <cerrno>
#endif

namespace platform {
#ifdef _WIN32
using Socket = SOCKET;
constexpr Socket invalid_socket = INVALID_SOCKET;
#else
using Socket = int;
constexpr Socket invalid_socket = -1;
#endif
using Count = std::ptrdiff_t;
inline int set_environment(const char *name,const char *value,int overwrite) {
    if(!overwrite && std::getenv(name))return 0;
#ifdef _WIN32
    return _putenv_s(name,value);
#else
    return setenv(name,value,1);
#endif
}
class Runtime {
public:
    Runtime() {
#ifdef _WIN32
        WSADATA data{};
        if(WSAStartup(MAKEWORD(2,2),&data))throw std::runtime_error("Winsock startup failed");
#endif
    }
    ~Runtime() {
#ifdef _WIN32
        WSACleanup();
#endif
    }
};
inline void close_socket(Socket socket) {
#ifdef _WIN32
    closesocket(socket);
#else
    close(socket);
#endif
}
inline void shutdown_socket(Socket socket) {
#ifdef _WIN32
    shutdown(socket,SD_BOTH);
#else
    shutdown(socket,SHUT_RDWR);
#endif
}
inline bool configure_media_socket(Socket socket) {
#ifdef _WIN32
    DWORD timeout=250;
    return setsockopt(socket,SOL_SOCKET,SO_SNDTIMEO,reinterpret_cast<const char*>(&timeout),sizeof(timeout))==0;
#else
#ifdef __APPLE__
    int one=1;if(setsockopt(socket,SOL_SOCKET,SO_NOSIGPIPE,&one,sizeof(one)))return false;
#endif
    timeval timeout{0,250000};
    return setsockopt(socket,SOL_SOCKET,SO_SNDTIMEO,&timeout,sizeof(timeout))==0;
#endif
}
inline Count send(Socket socket,const void *bytes,size_t length) {
    int flags=0;
#ifdef MSG_NOSIGNAL
    flags=MSG_NOSIGNAL;
#endif
#ifdef _WIN32
    return ::send(socket,static_cast<const char*>(bytes),static_cast<int>(length),flags);
#else
    return ::send(socket,bytes,length,flags);
#endif
}
#ifdef _WIN32
// The client never gives a pipe server impersonation rights. Verify the kernel
// server PID against the live parent, then its SID and the protected pipe ACL
// before exposing the startup capability token to the media channel.
class NativeHandle {
public:
    HANDLE value;
    explicit NativeHandle(HANDLE handle):value(handle){}
    NativeHandle(const NativeHandle&)=delete;
    NativeHandle& operator=(const NativeHandle&)=delete;
    ~NativeHandle(){if(value&&value!=INVALID_HANDLE_VALUE)CloseHandle(value);}
};
inline std::vector<uintptr_t> process_user(HANDLE process) {
    HANDLE handle=nullptr;
    if(!OpenProcessToken(process,TOKEN_QUERY,&handle))throw std::runtime_error("media IPC process token unavailable");
    NativeHandle token(handle);DWORD size=0;
    GetTokenInformation(token.value,TokenUser,nullptr,0,&size);
    if(!size||size>65536)throw std::runtime_error("media IPC process token invalid");
    std::vector<uintptr_t> user((size+sizeof(uintptr_t)-1)/sizeof(uintptr_t));
    if(!GetTokenInformation(token.value,TokenUser,user.data(),size,&size))throw std::runtime_error("media IPC process identity unavailable");
    return user;
}
inline DWORD parent_process_id() {
    NativeHandle snapshot(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS,0));
    PROCESSENTRY32W entry{};entry.dwSize=sizeof(entry);
    if(snapshot.value==INVALID_HANDLE_VALUE||!Process32FirstW(snapshot.value,&entry))return 0;
    do {if(entry.th32ProcessID==GetCurrentProcessId())return entry.th32ParentProcessID;}while(Process32NextW(snapshot.value,&entry));
    return 0;
}
class MediaPipe {
    HANDLE pipe=INVALID_HANDLE_VALUE;
    HANDLE write_event=nullptr;
public:
    MediaPipe()=default;
    MediaPipe(const MediaPipe&)=delete;
    MediaPipe& operator=(const MediaPipe&)=delete;
    ~MediaPipe(){if(connected())CloseHandle(pipe);if(write_event)CloseHandle(write_event);}
    bool connected() const {return pipe!=INVALID_HANDLE_VALUE;}
    void connect(const std::string &address) {
        const std::string prefix="\\\\.\\pipe\\NeonMix.Airplay.v1.";
        if(address.rfind(prefix,0)!=0||address.size()>256||address.size()==prefix.size()||
           address.find_first_not_of("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789.-",prefix.size())!=std::string::npos)
            throw std::runtime_error("invalid private media pipe");
        std::wstring name(address.begin(),address.end());
        pipe=CreateFileW(name.c_str(),GENERIC_WRITE|READ_CONTROL,0,nullptr,OPEN_EXISTING,
                         FILE_FLAG_OVERLAPPED|SECURITY_SQOS_PRESENT|SECURITY_IDENTIFICATION,nullptr);
        if(!connected())throw std::runtime_error("media IPC pipe connection failed");
        ULONG server_pid=0;DWORD parent_pid=parent_process_id();
        if(!parent_pid||!GetNamedPipeServerProcessId(pipe,&server_pid)||server_pid!=parent_pid)
            throw std::runtime_error("media IPC Hub identity mismatch");
        NativeHandle parent(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION,FALSE,parent_pid));
        FILETIME parent_created{},self_created{},exit_time{},kernel{},user_time{};
        // The recorded parent PID can be recycled after the parent dies. Its
        // current process must predate this worker before accepting that PID.
        if(!parent.value||!GetProcessTimes(parent.value,&parent_created,&exit_time,&kernel,&user_time)||
           !GetProcessTimes(GetCurrentProcess(),&self_created,&exit_time,&kernel,&user_time)||
           CompareFileTime(&parent_created,&self_created)>=0)
            throw std::runtime_error("media IPC Hub process mismatch");
        auto self=process_user(GetCurrentProcess()),peer=process_user(parent.value);
        PSID current=reinterpret_cast<TOKEN_USER*>(self.data())->User.Sid;
        if(!EqualSid(current,reinterpret_cast<TOKEN_USER*>(peer.data())->User.Sid))
            throw std::runtime_error("media IPC Hub user mismatch");
        PSECURITY_DESCRIPTOR descriptor=nullptr;PSID owner=nullptr;PACL acl=nullptr;
        if(GetSecurityInfo(pipe,SE_KERNEL_OBJECT,OWNER_SECURITY_INFORMATION|DACL_SECURITY_INFORMATION,
                           &owner,nullptr,&acl,nullptr,&descriptor)!=ERROR_SUCCESS)
            throw std::runtime_error("media IPC pipe security unavailable");
        SECURITY_DESCRIPTOR_CONTROL control=0;DWORD revision=0;
        bool valid=owner&&acl&&EqualSid(owner,current)&&GetSecurityDescriptorControl(descriptor,&control,&revision)&&(control&SE_DACL_PROTECTED);
        bool writable=false;
        for(DWORD i=0;valid&&i<acl->AceCount;i++){
            void *entry=nullptr;if(!GetAce(acl,i,&entry)){valid=false;break;}
            auto header=static_cast<ACE_HEADER*>(entry);
            if(header->AceFlags&INHERIT_ONLY_ACE)continue;
            if(header->AceType==ACCESS_DENIED_ACE_TYPE)continue;
            if(header->AceType!=ACCESS_ALLOWED_ACE_TYPE){valid=false;break;}
            auto ace=static_cast<ACCESS_ALLOWED_ACE*>(entry);
            if(!EqualSid(&ace->SidStart,current)){valid=false;break;}
            if(ace->Mask&(FILE_WRITE_DATA|GENERIC_WRITE|GENERIC_ALL))writable=true;
        }
        LocalFree(descriptor);
        if(!valid||!writable)throw std::runtime_error("media IPC pipe requires a protected owner-only DACL");
        DWORD mode=PIPE_READMODE_BYTE|PIPE_WAIT;
        if(!SetNamedPipeHandleState(pipe,&mode,nullptr,nullptr))throw std::runtime_error("media IPC pipe overlapped configuration failed");
        write_event=CreateEventW(nullptr,TRUE,FALSE,nullptr);
        if(!write_event)throw std::runtime_error("media IPC write event unavailable");
    }
    Count write(const void *bytes,size_t length,const std::atomic<bool> &running) {
        // Completion wakes the writer as soon as the Hub drains the pipe.
        // PIPE_NOWAIT + sleep_for(5ms) can quantize each packet to a 15.6ms
        // Windows scheduler tick, below 44.1kHz/352-frame audio throughput.
        // Keep one pending write and the same 250ms whole-packet budget.
        auto deadline=std::chrono::steady_clock::now()+std::chrono::milliseconds(250);
        size_t offset=0;
        while(offset<length&&running){
            if(!ResetEvent(write_event))return -1;
            OVERLAPPED operation{};operation.hEvent=write_event;
            DWORD count=0;
            if(!WriteFile(pipe,static_cast<const uint8_t*>(bytes)+offset,
                          static_cast<DWORD>(length-offset),&count,&operation)){
                if(GetLastError()!=ERROR_IO_PENDING)return -1;
                while(true){
                    const DWORD result=WaitForSingleObject(write_event,5);
                    if(result==WAIT_OBJECT_0)break;
                    if(result!=WAIT_TIMEOUT||!running||std::chrono::steady_clock::now()>=deadline){
                        CancelIoEx(pipe,&operation);
                        // Cancellation is asynchronous. Drain completion before
                        // freeing the packet or stack OVERLAPPED, even if the
                        // original write won the cancellation race.
                        GetOverlappedResult(pipe,&operation,&count,TRUE);
                        return -1;
                    }
                }
            }
            if(!GetOverlappedResult(pipe,&operation,&count,FALSE)||!count||count>length-offset)return -1;
            offset+=count;
            if(offset==length)return static_cast<Count>(offset);
            if(std::chrono::steady_clock::now()>=deadline)return -1;
        }
        return -1;
    }
};
#endif
inline void configure_control_output() {
#ifdef _WIN32
    DWORD mode=PIPE_READMODE_BYTE|PIPE_NOWAIT;
    HANDLE output=GetStdHandle(STD_OUTPUT_HANDLE);
    // The parent supplies pipes. Fail closed if immediate writes are unavailable;
    // do not replace this with a blocking CRT write or an unbounded event queue.
    if(GetFileType(output)!=FILE_TYPE_PIPE || !SetNamedPipeHandleState(output,&mode,nullptr,nullptr))
        throw std::runtime_error("stdout requires a nonblocking private pipe");
    if(GetFileType(GetStdHandle(STD_INPUT_HANDLE))!=FILE_TYPE_PIPE)
        throw std::runtime_error("stdin requires a private pipe");
#else
    int flags=fcntl(STDOUT_FILENO,F_GETFL);
    if(flags<0 || fcntl(STDOUT_FILENO,F_SETFL,flags|O_NONBLOCK)<0)
        throw std::runtime_error("stdout nonblocking configuration failed");
#endif
}
inline Count write_event(const void *bytes,size_t length) {
#ifdef _WIN32
    DWORD written=0;
    if(!WriteFile(GetStdHandle(STD_OUTPUT_HANDLE),bytes,static_cast<DWORD>(length),&written,nullptr))return -1;
    return written;
#else
    return write(STDOUT_FILENO,bytes,length);
#endif
}
// -1: failure/EOF, 0: timeout, positive: bytes. Never reads beyond ready data.
inline Count read_control(void *bytes,size_t length,int timeout_ms) {
#ifdef _WIN32
    auto deadline=std::chrono::steady_clock::now()+std::chrono::milliseconds(timeout_ms);
    HANDLE input=GetStdHandle(STD_INPUT_HANDLE);
    do {
        DWORD available=0;
        if(!PeekNamedPipe(input,nullptr,0,nullptr,&available,nullptr))return -1;
        if(available){DWORD count=0;
            if(!ReadFile(input,bytes,static_cast<DWORD>(std::min<size_t>(available,length)),&count,nullptr)||!count)return -1;
            return count;
        }
        if(std::chrono::steady_clock::now()>=deadline)return 0;
        std::this_thread::sleep_for(std::chrono::milliseconds(5));
    }while(true);
#else
    pollfd descriptor{STDIN_FILENO,POLLIN,0};
    int result=poll(&descriptor,1,timeout_ms);
    if(result<0)return errno==EINTR?0:-1;
    if(!result)return 0;
    if(descriptor.revents&(POLLERR|POLLNVAL))return -1;
    Count count=read(STDIN_FILENO,bytes,length);
    return count>0?count:-1;
#endif
}
// Fixed-size, noncopyable secret storage. Its destructor also runs on read and
// parse failures; volatile writes cannot be optimized away.
class PrivateKey {
public:
    std::array<unsigned char,4096> bytes{};
    size_t size=0;
    PrivateKey()=default;
    PrivateKey(const PrivateKey&)=delete;
    PrivateKey& operator=(const PrivateKey&)=delete;
    void clear() noexcept {
        volatile unsigned char *data=bytes.data();
        for(size_t i=0;i<bytes.size();i++)data[i]=0;
        size=0;
    }
    ~PrivateKey(){clear();}
};
inline bool valid_utf8(const std::string &value) {
    for(size_t i=0;i<value.size();){
        auto first=static_cast<unsigned char>(value[i++]);
        if(first<0x80)continue;
        unsigned count=first>=0xc2&&first<=0xdf?1:first>=0xe0&&first<=0xef?2:first>=0xf0&&first<=0xf4?3:0;
        if(!count||i+count>value.size())return false;
        uint32_t code=first&((1u<<(6-count))-1);
        for(unsigned j=0;j<count;j++){
            auto next=static_cast<unsigned char>(value[i++]);
            if((next&0xc0)!=0x80)return false;
            code=(code<<6)|(next&0x3f);
        }
        if(code<(count==1?0x80u:count==2?0x800u:0x10000u)||code>0x10ffff||(code>=0xd800&&code<=0xdfff))return false;
    }
    return true;
}
inline std::unique_ptr<PrivateKey> read_private_key(const std::string &path
#ifdef NEONMIX_IDENTITY_PROBE
    , void (*validated_hook)(const std::string&)=nullptr
#endif
) {
    if(path.empty() || path.find('\0')!=std::string::npos)throw std::runtime_error("identity_path_invalid");
    if(!valid_utf8(path))throw std::runtime_error("identity_path_encoding");
    auto key=std::make_unique<PrivateKey>();
#ifdef _WIN32
    int size=MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,path.c_str(),-1,nullptr,0);
    if(!size)throw std::runtime_error("identity_path_encoding");
    std::vector<wchar_t> wide(size);
    if(!MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,path.c_str(),-1,wide.data(),size))
        throw std::runtime_error("identity_path_encoding");
    size_t prefix=path.rfind("\\\\?\\",0)==0?4:0;
    size_t drive=(path.size()>prefix+1 && path[prefix+1]==':')?prefix+2:0;
    if(path.find(':',drive)!=std::string::npos)throw std::runtime_error("identity_path_invalid");
    DWORD count=GetFullPathNameW(wide.data(),0,nullptr,nullptr);
    if(!count)throw std::runtime_error("identity_path_invalid");
    std::vector<wchar_t> full(count);
    if(!GetFullPathNameW(wide.data(),count,full.data(),nullptr))throw std::runtime_error("identity_path_invalid");
    // Pin ancestors top down without delete-sharing, matching identity/files_windows.
    // OPEN_REPARSE_POINT on the final file alone does not check junction parents.
    auto absolute=std::filesystem::path(full.data());
    std::vector<std::filesystem::path> parents;
    for(auto parent=absolute.parent_path();!parent.empty();){
        parents.push_back(parent);
        auto next=parent.parent_path();if(next==parent)break;parent=next;
    }
    std::vector<std::unique_ptr<NativeHandle>> pinned;
    for(auto it=parents.rbegin();it!=parents.rend();++it){
        auto handle=std::make_unique<NativeHandle>(CreateFileW(it->c_str(),FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ|FILE_SHARE_WRITE,nullptr,OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT|FILE_FLAG_BACKUP_SEMANTICS,nullptr));
        BY_HANDLE_FILE_INFORMATION info{};
        if(handle->value==INVALID_HANDLE_VALUE || !GetFileInformationByHandle(handle->value,&info) ||
           !(info.dwFileAttributes&FILE_ATTRIBUTE_DIRECTORY) || (info.dwFileAttributes&FILE_ATTRIBUTE_REPARSE_POINT))
            throw std::runtime_error("identity_permission_denied");
        pinned.push_back(std::move(handle));
    }
    NativeHandle opened(CreateFileW(full.data(),GENERIC_READ|READ_CONTROL,FILE_SHARE_READ,nullptr,OPEN_EXISTING,FILE_FLAG_OPEN_REPARSE_POINT,nullptr));
    HANDLE file=opened.value;
    if(file==INVALID_HANDLE_VALUE){
        DWORD error=GetLastError();
        throw std::runtime_error(error==ERROR_FILE_NOT_FOUND||error==ERROR_PATH_NOT_FOUND?
            "identity_not_found":error==ERROR_ACCESS_DENIED?"identity_permission_denied":"identity_read_failed");
    }
    bool valid=false;PSECURITY_DESCRIPTOR descriptor=nullptr;HANDLE token=nullptr;
    do {
        BY_HANDLE_FILE_INFORMATION info{};DWORD volume_flags=0;
        if(GetFileType(file)!=FILE_TYPE_DISK || !GetFileInformationByHandle(file,&info) ||
           (info.dwFileAttributes&(FILE_ATTRIBUTE_DIRECTORY|FILE_ATTRIBUTE_REPARSE_POINT)) ||
           !GetVolumeInformationByHandleW(file,nullptr,0,nullptr,nullptr,&volume_flags,nullptr,0) || !(volume_flags&FILE_PERSISTENT_ACLS))break;
        PSID owner=nullptr;PACL acl=nullptr;
        if(GetSecurityInfo(file,SE_FILE_OBJECT,OWNER_SECURITY_INFORMATION|DACL_SECURITY_INFORMATION,&owner,nullptr,&acl,nullptr,&descriptor)!=ERROR_SUCCESS || !owner || !acl)break;
        SECURITY_DESCRIPTOR_CONTROL control=0;DWORD revision=0;
        if(!GetSecurityDescriptorControl(descriptor,&control,&revision) || !(control&SE_DACL_PROTECTED))break;
        if(!OpenProcessToken(GetCurrentProcess(),TOKEN_QUERY,&token))break;
        DWORD needed=0;GetTokenInformation(token,TokenUser,nullptr,0,&needed);
        std::vector<uint8_t> user(needed);
        if(!needed || !GetTokenInformation(token,TokenUser,user.data(),needed,&needed))break;
        PSID current=reinterpret_cast<TOKEN_USER*>(user.data())->User.Sid;
        if(!EqualSid(owner,current))break;
        SID_IDENTIFIER_AUTHORITY authority=SECURITY_CREATOR_SID_AUTHORITY;PSID owner_rights=nullptr;
        if(!AllocateAndInitializeSid(&authority,1,SECURITY_CREATOR_OWNER_RIGHTS_RID,0,0,0,0,0,0,0,&owner_rights))break;
        valid=true;bool readable=false;
        for(DWORD i=0;i<acl->AceCount;i++){
            void *entry=nullptr;if(!GetAce(acl,i,&entry)){valid=false;break;}
            auto header=static_cast<ACE_HEADER*>(entry);
            if(header->AceFlags&INHERIT_ONLY_ACE)continue;
            if(header->AceType==ACCESS_DENIED_ACE_TYPE)continue;
            if(header->AceType!=ACCESS_ALLOWED_ACE_TYPE){valid=false;break;}
            auto ace=static_cast<ACCESS_ALLOWED_ACE*>(entry);PSID sid=&ace->SidStart;
            if(!EqualSid(sid,current) && !EqualSid(sid,owner_rights)){valid=false;break;}
            if(ace->Mask&(FILE_READ_DATA|GENERIC_READ|GENERIC_ALL))readable=true;
        }
        FreeSid(owner_rights);valid=valid&&readable;
    }while(false);
    if(token)CloseHandle(token);if(descriptor)LocalFree(descriptor);
    if(!valid)throw std::runtime_error("identity_permission_denied");

#ifdef NEONMIX_IDENTITY_PROBE
    if(validated_hook)validated_hook(path);
#endif
    while(key->size<key->bytes.size()){
        DWORD read=0;
        if(!ReadFile(file,key->bytes.data()+key->size,static_cast<DWORD>(key->bytes.size()-key->size),&read,nullptr))
            throw std::runtime_error("identity_read_failed");
        if(!read)break;
        key->size+=read;
    }
#else
    struct Descriptor {
        int value;
        ~Descriptor(){if(value>=0)close(value);}
    } opened{open(path.c_str(),O_RDONLY|O_NOFOLLOW|O_NONBLOCK|O_CLOEXEC)};
    if(opened.value<0)throw std::runtime_error(errno==ENOENT?"identity_not_found":
        errno==EACCES||errno==ELOOP?"identity_permission_denied":"identity_read_failed");
    struct stat info{};
    if(fstat(opened.value,&info)!=0||!S_ISREG(info.st_mode)||info.st_uid!=geteuid()||(info.st_mode&0777)!=0600)
        throw std::runtime_error("identity_permission_denied");
#ifdef NEONMIX_IDENTITY_PROBE
    if(validated_hook)validated_hook(path);
#endif
    while(key->size<key->bytes.size()){
        auto read=::read(opened.value,key->bytes.data()+key->size,key->bytes.size()-key->size);
        if(read<0){if(errno==EINTR)continue;throw std::runtime_error("identity_read_failed");}
        if(!read)break;
        key->size+=static_cast<size_t>(read);
    }
#endif
    if(!key->size || key->size>=key->bytes.size())throw std::runtime_error("identity_format_invalid");
    return key;
}

} // namespace platform
#ifdef _WIN32
// The optional decoder probe includes main.cpp and uses this POSIX spelling.
inline int setenv(const char *name,const char *value,int overwrite) {
    return platform::set_environment(name,value,overwrite);
}
#endif
