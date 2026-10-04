// SPDX-License-Identifier: GPL-3.0-or-later
// Only local process IPC and private-file checks belong in this adapter.
#pragma once
#include <algorithm>
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
public:
    MediaPipe()=default;
    MediaPipe(const MediaPipe&)=delete;
    MediaPipe& operator=(const MediaPipe&)=delete;
    ~MediaPipe(){if(connected())CloseHandle(pipe);}
    bool connected() const {return pipe!=INVALID_HANDLE_VALUE;}
    void connect(const std::string &address) {
        const std::string prefix="\\\\.\\pipe\\NeonMix.Airplay.v1.";
        if(address.rfind(prefix,0)!=0||address.size()>256||address.size()==prefix.size()||
           address.find_first_not_of("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789.-",prefix.size())!=std::string::npos)
            throw std::runtime_error("invalid private media pipe");
        std::wstring name(address.begin(),address.end());
        pipe=CreateFileW(name.c_str(),GENERIC_WRITE|READ_CONTROL,0,nullptr,OPEN_EXISTING,
                         SECURITY_SQOS_PRESENT|SECURITY_IDENTIFICATION,nullptr);
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
        DWORD mode=PIPE_READMODE_BYTE|PIPE_NOWAIT;
        if(!SetNamedPipeHandleState(pipe,&mode,nullptr,nullptr))throw std::runtime_error("media IPC pipe nonblocking configuration failed");
    }
    Count write(const void *bytes,size_t length,const std::atomic<bool> &running) {
        // PIPE_NOWAIT returns immediately, including zero/partial byte writes
        // under backpressure. One entire packet has a 250 ms budget; stop is
        // observed between calls, with at most a 5 ms poll delay and no pending IO.
        auto deadline=std::chrono::steady_clock::now()+std::chrono::milliseconds(250);
        size_t offset=0;
        while(offset<length&&running){
            DWORD count=0;
            if(!WriteFile(pipe,static_cast<const uint8_t*>(bytes)+offset,
                          static_cast<DWORD>(length-offset),&count,nullptr))return -1;
            offset+=count;
            if(offset==length)return static_cast<Count>(offset);
            if(std::chrono::steady_clock::now()>=deadline)return -1;
            if(!count)std::this_thread::sleep_for(std::chrono::milliseconds(5));
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
inline std::string private_key_path(const std::string &path) {
    if(path.empty() || path.find('\0')!=std::string::npos)throw std::runtime_error("invalid private key path");
#ifdef _WIN32
    // UTF-8 comes from JSON; Win32 ACL checks use wide paths. UxPlay fopen uses
    // the process ANSI code page: reject lossy conversion rather than open a
    // different key. Relocatable UTF-8 packaging remains a separate obligation.
    int size=MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,path.c_str(),-1,nullptr,0);
    if(!size)throw std::runtime_error("invalid UTF-8 private key path");
    std::vector<wchar_t> wide(size);
    MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,path.c_str(),-1,wide.data(),size);
    size_t prefix=path.rfind("\\\\?\\",0)==0?4:0;
    size_t drive=(path.size()>prefix+1 && path[prefix+1]==':')?prefix+2:0;
    if(path.find(':',drive)!=std::string::npos)throw std::runtime_error("private key alternate data stream refused");
    HANDLE file=CreateFileW(wide.data(),GENERIC_READ|READ_CONTROL,FILE_SHARE_READ,nullptr,OPEN_EXISTING,FILE_FLAG_OPEN_REPARSE_POINT,nullptr);
    if(file==INVALID_HANDLE_VALUE)throw std::runtime_error("private key open failed");
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
    if(token)CloseHandle(token);if(descriptor)LocalFree(descriptor);CloseHandle(file);
    if(!valid)throw std::runtime_error("Hub private key requires a current-user regular file with a protected owner-only DACL");
    BOOL lossy=FALSE;UINT codepage=GetACP();DWORD flags=codepage==CP_UTF8?WC_ERR_INVALID_CHARS:WC_NO_BEST_FIT_CHARS;
    BOOL *used_default=codepage==CP_UTF8?nullptr:&lossy;
    int narrow_size=WideCharToMultiByte(codepage,flags,wide.data(),-1,nullptr,0,nullptr,used_default);
    if(!narrow_size||lossy)throw std::runtime_error("private key path cannot be represented by protocol file API");
    std::string narrow(narrow_size,'\0');
    if(!WideCharToMultiByte(codepage,flags,wide.data(),-1,narrow.data(),narrow_size,nullptr,used_default)||lossy)
        throw std::runtime_error("private key path conversion failed");
    narrow.pop_back();return narrow;
#else
    struct stat info{};
    if(lstat(path.c_str(),&info)!=0||!S_ISREG(info.st_mode)||info.st_uid!=getuid()||(info.st_mode&0777)!=0600)
        throw std::runtime_error("Hub private key must be current-user regular file with mode 0600");
    return path;
#endif
}
} // namespace platform
#ifdef _WIN32
// The optional decoder probe includes main.cpp and uses this POSIX spelling.
inline int setenv(const char *name,const char *value,int overwrite) {
    return platform::set_environment(name,value,overwrite);
}
#endif
