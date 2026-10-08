// Closed per-installation firewall maintenance. No shell, arbitrary program or
// rule input. Package hashes are compiled in after staging the two socket owners.
#define UNICODE
#define _UNICODE
#define NOMINMAX
#include <windows.h>
#include <netfw.h>
#include <netlistmgr.h>
#include <sddl.h>
#include <shellapi.h>
#include <bcrypt.h>
#include <objbase.h>
#include <algorithm>
#include <array>
#include <filesystem>
#include <cwctype>
#include <iostream>
#include <memory>
#include <map>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>
#include "network_package.h"

namespace fs=std::filesystem;
static void check(HRESULT hr){if(FAILED(hr))throw std::runtime_error("network_api_failed");}
struct Handle {
    HANDLE value;
    explicit Handle(HANDLE h):value(h){}
    Handle(const Handle&)=delete;
    ~Handle(){if(value&&value!=INVALID_HANDLE_VALUE)CloseHandle(value);}
};
template<class T> struct Com {
    T* value=nullptr;
    Com()=default;
    Com(const Com&)=delete;
    ~Com(){if(value)value->Release();}
    T* operator->()const{return value;}
};
struct Text {
    BSTR value;
    explicit Text(const std::wstring& s):value(SysAllocStringLen(s.data(),static_cast<UINT>(s.size()))){if(!value)throw std::bad_alloc();}
    ~Text(){SysFreeString(value);}
};
static std::wstring sid(HANDLE process){
    HANDLE raw=nullptr;if(!OpenProcessToken(process,TOKEN_QUERY,&raw))throw std::runtime_error("network_identity_unavailable");
    Handle token(raw);DWORD size=0;GetTokenInformation(raw,TokenUser,nullptr,0,&size);
    if(!size||size>65536)throw std::runtime_error("network_identity_invalid");
    std::vector<ULONG_PTR> data((size+sizeof(ULONG_PTR)-1)/sizeof(ULONG_PTR));
    if(!GetTokenInformation(raw,TokenUser,data.data(),size,&size))throw std::runtime_error("network_identity_unavailable");
    wchar_t* text=nullptr;if(!ConvertSidToStringSidW(reinterpret_cast<TOKEN_USER*>(data.data())->User.Sid,&text))throw std::runtime_error("network_identity_unavailable");
    std::wstring result(text);LocalFree(text);return result;
}
static bool elevated(){
    HANDLE raw=nullptr;if(!OpenProcessToken(GetCurrentProcess(),TOKEN_QUERY,&raw))throw std::runtime_error("network_token_unavailable");
    Handle token(raw);TOKEN_ELEVATION elevation{};DWORD size=0;
    if(!GetTokenInformation(raw,TokenElevation,&elevation,sizeof(elevation),&size))throw std::runtime_error("network_token_unavailable");
    return elevation.TokenIsElevated!=0;
}
static fs::path executable(){
    std::vector<wchar_t> path(32768);
    DWORD length=GetModuleFileNameW(nullptr,path.data(),static_cast<DWORD>(path.size()));
    if(!length||length>=path.size())throw std::runtime_error("network_module_unavailable");
    return fs::path(std::wstring(path.data(),length));
}
static std::unique_ptr<Handle> open(const fs::path& path,bool directory){
    auto file=std::make_unique<Handle>(CreateFileW(path.c_str(),directory?FILE_READ_ATTRIBUTES:GENERIC_READ,
        directory?FILE_SHARE_READ|FILE_SHARE_WRITE:FILE_SHARE_READ,nullptr,OPEN_EXISTING,
        FILE_FLAG_OPEN_REPARSE_POINT|(directory?FILE_FLAG_BACKUP_SEMANTICS:0),nullptr));
    BY_HANDLE_FILE_INFORMATION info{};
    if(file->value==INVALID_HANDLE_VALUE||GetFileType(file->value)!=FILE_TYPE_DISK||!GetFileInformationByHandle(file->value,&info)||
       (info.dwFileAttributes&FILE_ATTRIBUTE_REPARSE_POINT)||bool(info.dwFileAttributes&FILE_ATTRIBUTE_DIRECTORY)!=directory)
        throw std::runtime_error("network_package_path_invalid");
    return file;
}
static std::vector<std::unique_ptr<Handle>> pin(const fs::path& path){
    std::vector<fs::path> parents;
    for(auto parent=path;!parent.empty();){parents.push_back(parent);auto next=parent.parent_path();if(next==parent)break;parent=next;}
    std::vector<std::unique_ptr<Handle>> handles;
    for(auto it=parents.rbegin();it!=parents.rend();++it)handles.push_back(open(*it,true));
    return handles;
}
static bool same_file(HANDLE a,HANDLE b){
    BY_HANDLE_FILE_INFORMATION x{},y{};
    return GetFileInformationByHandle(a,&x)&&GetFileInformationByHandle(b,&y)&&
        x.dwVolumeSerialNumber==y.dwVolumeSerialNumber&&x.nFileIndexHigh==y.nFileIndexHigh&&x.nFileIndexLow==y.nFileIndexLow;
}
static std::string digest(HANDLE file){
    BCRYPT_ALG_HANDLE algorithm=nullptr;BCRYPT_HASH_HANDLE hash=nullptr;
    if(BCryptOpenAlgorithmProvider(&algorithm,BCRYPT_SHA256_ALGORITHM,nullptr,0)<0)throw std::runtime_error("network_hash_unavailable");
    struct Cleanup {BCRYPT_ALG_HANDLE& a;BCRYPT_HASH_HANDLE& h;~Cleanup(){if(h)BCryptDestroyHash(h);if(a)BCryptCloseAlgorithmProvider(a,0);}}cleanup{algorithm,hash};
    if(BCryptCreateHash(algorithm,&hash,nullptr,0,nullptr,0,0)<0)throw std::runtime_error("network_hash_unavailable");
    std::array<unsigned char,65536> buffer{};DWORD count=0;
    for(;;){if(!ReadFile(file,buffer.data(),static_cast<DWORD>(buffer.size()),&count,nullptr))throw std::runtime_error("network_package_read_failed");if(!count)break;
        if(BCryptHashData(hash,buffer.data(),count,0)<0)throw std::runtime_error("network_hash_unavailable");}
    std::array<unsigned char,32> bytes{};if(BCryptFinishHash(hash,bytes.data(),32,0)<0)throw std::runtime_error("network_hash_unavailable");
    static const char hex[]="0123456789abcdef";std::string result;
    for(auto byte:bytes){result+=hex[byte>>4];result+=hex[byte&15];}return result;
}
// Persist only through a validated, held handle. Path-based INI writes after
// elevation could follow a substituted link, and ANSI INI files lose Unicode.
class ConfigFile {
    std::unique_ptr<Handle> file;
    std::map<std::wstring,std::wstring> fields;
    bool writable;
public:
    ConfigFile(const fs::path& path,bool write,const std::wstring& owner):writable(write){
        PSECURITY_DESCRIPTOR security=nullptr;
        std::wstring sddl=L"D:P(A;;FA;;;"+owner+L")(A;;FA;;;BA)(A;;FA;;;SY)";
        if(!ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.c_str(),SDDL_REVISION_1,&security,nullptr))throw std::runtime_error("network_record_invalid");
        SECURITY_ATTRIBUTES attributes{sizeof(attributes),security,FALSE};
        HANDLE handle=CreateFileW(path.c_str(),GENERIC_READ|(write?GENERIC_WRITE:0),write?FILE_SHARE_READ:FILE_SHARE_READ|FILE_SHARE_WRITE,
            &attributes,write?OPEN_ALWAYS:OPEN_EXISTING,FILE_FLAG_OPEN_REPARSE_POINT,nullptr);
        LocalFree(security);file=std::make_unique<Handle>(handle);
        BY_HANDLE_FILE_INFORMATION info{};
        if(handle==INVALID_HANDLE_VALUE||GetFileType(handle)!=FILE_TYPE_DISK||!GetFileInformationByHandle(handle,&info)||
            (info.dwFileAttributes&(FILE_ATTRIBUTE_REPARSE_POINT|FILE_ATTRIBUTE_DIRECTORY))||info.nNumberOfLinks!=1||info.nFileSizeHigh||info.nFileSizeLow>65536)
            throw std::runtime_error("network_record_invalid");
        std::vector<unsigned char> bytes(info.nFileSizeLow);DWORD count=0;
        if(!bytes.empty()&&(!ReadFile(handle,bytes.data(),static_cast<DWORD>(bytes.size()),&count,nullptr)||count!=bytes.size()))throw std::runtime_error("network_record_invalid");
        std::wstring text;
        if(bytes.size()>=2&&bytes[0]==0xff&&bytes[1]==0xfe){
            if(bytes.size()%2)throw std::runtime_error("network_record_invalid");
            for(size_t i=2;i<bytes.size();i+=2)text+=static_cast<wchar_t>(bytes[i]|(unsigned(bytes[i+1])<<8));
        }else{
            for(auto byte:bytes){if(byte>=128)throw std::runtime_error("network_record_invalid");text+=wchar_t(byte);}
        }
        std::wistringstream input(text);std::wstring line;
        while(std::getline(input,line)){
            if(!line.empty()&&line.back()==L'\r')line.pop_back();
            if(line.empty()||line==L"[NeonMix]")continue;
            auto equals=line.find(L'=');
            if(equals==std::wstring::npos||equals==0||fields.count(line.substr(0,equals)))throw std::runtime_error("network_record_invalid");
            fields.emplace(line.substr(0,equals),line.substr(equals+1));
        }
    }
    std::wstring get(const std::wstring& key)const{auto value=fields.find(key);return value==fields.end()?L"":value->second;}
    void set(const std::wstring& key,const std::wstring& value){
        if(!writable||key.find_first_of(L"=\r\n")!=std::wstring::npos||value.find_first_of(L"\r\n")!=std::wstring::npos)throw std::runtime_error("network_record_write_failed");
        fields[key]=value;std::wstring text=L"\ufeff[NeonMix]\r\n";
        for(const auto& field:fields)text+=field.first+L"="+field.second+L"\r\n";
        if(text.size()*sizeof(wchar_t)>65536)throw std::runtime_error("network_record_write_failed");
        LARGE_INTEGER start{};DWORD written=0;
        if(!SetFilePointerEx(file->value,start,nullptr,FILE_BEGIN)||!WriteFile(file->value,text.data(),static_cast<DWORD>(text.size()*sizeof(wchar_t)),&written,nullptr)||
           written!=text.size()*sizeof(wchar_t)||!SetEndOfFile(file->value)||!FlushFileBuffers(file->value))throw std::runtime_error("network_record_write_failed");
    }
};
static std::wstring guid(){GUID value{};check(CoCreateGuid(&value));wchar_t text[40]{};if(!StringFromGUID2(value,text,40))throw std::runtime_error("network_instance_invalid");
    std::wstring result(text+1,36);std::transform(result.begin(),result.end(),result.begin(),::towlower);return result;}
static bool valid_guid(const std::wstring& value){
    if(value.size()!=36)return false;
    for(size_t i=0;i<value.size();i++)if(i==8||i==13||i==18||i==23){if(value[i]!='-')return false;}else if(!((value[i]>='0'&&value[i]<='9')||(value[i]>='a'&&value[i]<='f')))return false;
    return true;
}
struct Template {const wchar_t* suffix;fs::path program;long protocol;std::wstring ports;};
static std::array<Template,4> templates(const fs::path& root,unsigned port){return {{
    {L"Hub-Control-TCP",(root/L"bin/neonmix-hub.exe").make_preferred(),NET_FW_IP_PROTOCOL_TCP,std::to_wstring(port)},
    {L"Hub-Media-UDP",(root/L"bin/neonmix-hub.exe").make_preferred(),NET_FW_IP_PROTOCOL_UDP,L"*"},
    {L"AirPlay-Control-TCP",(root/L"airplay/bin/neonmix-airplay-worker.exe").make_preferred(),NET_FW_IP_PROTOCOL_TCP,L"*"},
    {L"AirPlay-Media-UDP",(root/L"airplay/bin/neonmix-airplay-worker.exe").make_preferred(),NET_FW_IP_PROTOCOL_UDP,L"*"}
}};}
static std::wstring read_text(INetFwRule* rule,HRESULT(STDMETHODCALLTYPE INetFwRule::*get)(BSTR*)){BSTR raw=nullptr;check((rule->*get)(&raw));std::wstring result(raw?raw:L"");SysFreeString(raw);return result;}
static bool matches(INetFwRule* rule,const Template& expected,const std::wstring& group){
    long protocol=0,profiles=0;NET_FW_RULE_DIRECTION direction{};NET_FW_ACTION action{};VARIANT_BOOL enabled=0,edge=0;
    check(rule->get_Protocol(&protocol));check(rule->get_Profiles(&profiles));check(rule->get_Direction(&direction));check(rule->get_Action(&action));check(rule->get_Enabled(&enabled));check(rule->get_EdgeTraversal(&edge));
    VARIANT interfaces;VariantInit(&interfaces);check(rule->get_Interfaces(&interfaces));bool all_interfaces=interfaces.vt==VT_EMPTY;VariantClear(&interfaces);
    return protocol==expected.protocol&&profiles==7&&direction==NET_FW_RULE_DIR_IN&&action==NET_FW_ACTION_ALLOW&&enabled==VARIANT_TRUE&&!edge&&all_interfaces&&
        _wcsicmp(read_text(rule,&INetFwRule::get_ApplicationName).c_str(),expected.program.c_str())==0&&read_text(rule,&INetFwRule::get_LocalPorts)==expected.ports&&
        read_text(rule,&INetFwRule::get_RemotePorts)==L"*"&&read_text(rule,&INetFwRule::get_LocalAddresses)==L"*"&&
        read_text(rule,&INetFwRule::get_RemoteAddresses)==L"LocalSubnet"&&read_text(rule,&INetFwRule::get_Grouping)==group&&
        read_text(rule,&INetFwRule::get_ServiceName).empty()&&read_text(rule,&INetFwRule::get_InterfaceTypes)==L"All";
}
static void fill(INetFwRule* rule,const Template& expected,const std::wstring& name,const std::wstring& group,const std::wstring& attribution){
    Text n(name),g(group),description(L"NeonMix " NEONMIX_PACKAGE_VERSION +attribution),program(expected.program.wstring()),ports(expected.ports),any(L"*"),subnet(L"LocalSubnet"),all(L"All");
    check(rule->put_Name(n.value));check(rule->put_Grouping(g.value));check(rule->put_Description(description.value));check(rule->put_ApplicationName(program.value));
    check(rule->put_Protocol(expected.protocol));check(rule->put_LocalPorts(ports.value));check(rule->put_RemotePorts(any.value));check(rule->put_LocalAddresses(any.value));check(rule->put_RemoteAddresses(subnet.value));
    check(rule->put_Profiles(7));check(rule->put_Direction(NET_FW_RULE_DIR_IN));check(rule->put_Action(NET_FW_ACTION_ALLOW));check(rule->put_InterfaceTypes(all.value));
    check(rule->put_Enabled(VARIANT_TRUE));check(rule->put_EdgeTraversal(VARIANT_FALSE));
}
// 0 configured, 1 missing, 2 path mismatch, 3 policy blocked, 4 unknown.
struct Inspection {int configuration=0,effective=4;long profiles=0;bool blocked=false,matching_block=false,categories_known=false;std::vector<int> categories;std::array<bool,4> rules{};std::array<int,3> enabled{{-1,-1,-1}};};
static Inspection inspect(INetFwPolicy2* policy,INetFwRules* rules,const std::array<Template,4>& expected,const std::wstring& group){
    Inspection status;check(policy->get_CurrentProfileTypes(&status.profiles));
    Com<INetworkListManager> networks;
    if(SUCCEEDED(CoCreateInstance(__uuidof(NetworkListManager),nullptr,CLSCTX_INPROC_SERVER,__uuidof(INetworkListManager),reinterpret_cast<void**>(&networks.value)))) {
        Com<IEnumNetworks> connected;
        if(SUCCEEDED(networks->GetNetworks(NLM_ENUM_NETWORK_CONNECTED,&connected.value))) {
            status.categories_known=true;
            for(unsigned i=0;i<256;i++) {
                Com<INetwork> network;ULONG count=0;
                HRESULT result=connected->Next(1,&network.value,&count);
                if(result==S_FALSE)break;
                if(FAILED(result)||!network.value){status.categories_known=false;break;}
                NLM_NETWORK_CATEGORY category{};
                if(FAILED(network->GetCategory(&category))){status.categories_known=false;break;}
                status.categories.push_back(static_cast<int>(category));
                if(i==255)status.categories_known=false;
            }
        }
    }
    NET_FW_MODIFY_STATE modify{};if(SUCCEEDED(policy->get_LocalPolicyModifyState(&modify))&&modify!=NET_FW_MODIFY_STATE_OK)status.blocked=true;
    const NET_FW_PROFILE_TYPE2 profiles[]={NET_FW_PROFILE2_DOMAIN,NET_FW_PROFILE2_PRIVATE,NET_FW_PROFILE2_PUBLIC};
    for(size_t i=0;i<3;i++){VARIANT_BOOL enabled=0,block=0;if(SUCCEEDED(policy->get_FirewallEnabled(profiles[i],&enabled)))status.enabled[i]=enabled?1:0;
        if(status.enabled[i]==1&&(status.profiles&profiles[i])&&SUCCEEDED(policy->get_BlockAllInboundTraffic(profiles[i],&block))&&block)status.blocked=true;}
    for(size_t i=0;i<expected.size();i++){
        Text name(group+L"."+expected[i].suffix);Com<INetFwRule> rule;
        HRESULT hr=rules->Item(name.value,&rule.value);
        if(FAILED(hr)){if(hr!=HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND))check(hr);status.configuration=std::max(status.configuration,1);continue;}
        if(_wcsicmp(read_text(rule.value,&INetFwRule::get_ApplicationName).c_str(),expected[i].program.c_str())!=0)status.configuration=2;
        status.rules[i]=matches(rule.value,expected[i],group);
        if(!status.rules[i]&&status.configuration==0)status.configuration=1;
    }
    long enforced_profiles=0;for(size_t i=0;i<3;i++)if(status.enabled[i]==1)enforced_profiles|=profiles[i];
    Com<IUnknown> unknown;check(rules->get__NewEnum(&unknown.value));Com<IEnumVARIANT> enumerator;check(unknown->QueryInterface(IID_IEnumVARIANT,reinterpret_cast<void**>(&enumerator.value)));
    for(;;){VARIANT item;VariantInit(&item);ULONG count=0;HRESULT hr=enumerator->Next(1,&item,&count);
        if(hr==S_FALSE){VariantClear(&item);break;}check(hr);Com<INetFwRule> rule;
        if(item.vt==VT_DISPATCH)check(item.pdispVal->QueryInterface(__uuidof(INetFwRule),reinterpret_cast<void**>(&rule.value)));
        VariantClear(&item);if(!rule.value)continue;
        NET_FW_ACTION action{};NET_FW_RULE_DIRECTION direction{};VARIANT_BOOL enabled=0;long active=0;
        check(rule->get_Action(&action));check(rule->get_Direction(&direction));check(rule->get_Enabled(&enabled));check(rule->get_Profiles(&active));
        if(action!=NET_FW_ACTION_BLOCK||direction!=NET_FW_RULE_DIR_IN||!enabled||!(active&status.profiles))continue;
        auto program=read_text(rule.value,&INetFwRule::get_ApplicationName);
        // Conservatively report matching explicit blocks; never remove them.
        if(program.empty()||std::any_of(expected.begin(),expected.end(),[&](const auto& t){return _wcsicmp(program.c_str(),t.program.c_str())==0;})){status.matching_block=true;if(active&status.profiles&enforced_profiles)status.blocked=true;}
    }
    if(status.blocked)status.effective=3;
    // COM cannot prove third-party/WFP or every GPO condition. A matching local
    // Allow is reported separately; effective policy stays unknown unless blocked.
    return status;
}
static void report(const Inspection& s){
    static const char* names[]={"configured","missing","path_mismatch","policy_blocked","unknown"};
    std::cout<<"{\"configuration\":\""<<names[s.configuration]<<"\",\"configuration_code\":"<<s.configuration<<",\"effective_policy\":\""<<names[s.effective]<<"\",\"effective_policy_code\":"<<s.effective<<",\"network_ready\":false,\"active_profiles\":"<<s.profiles<<",\"matching_block_rule\":"<<(s.matching_block?"true":"false")<<",\"firewall_enabled\":[";
    for(size_t i=0;i<3;i++){if(i)std::cout<<",";std::cout<<s.enabled[i];}std::cout<<"],\"network_categories_known\":"<<(s.categories_known?"true":"false")<<",\"network_categories\":[";
    for(size_t i=0;i<s.categories.size();i++){if(i)std::cout<<",";std::cout<<s.categories[i];}std::cout<<"],\"rules_match\":[";
    for(size_t i=0;i<4;i++){if(i)std::cout<<",";std::cout<<(s.rules[i]?"true":"false");}std::cout<<"]}\n";
}
static int run(const std::wstring& operation,unsigned port,DWORD requester){
    const auto module=executable(),root=module.parent_path().parent_path();
    if(_wcsicmp(module.filename().c_str(),L"neonmix-network-helper.exe")||_wcsicmp(module.parent_path().filename().c_str(),L"bin"))throw std::runtime_error("network_installation_invalid");
    auto ancestors=pin(root);auto bin=pin(root/L"bin"),worker_bin=pin(root/L"airplay/bin");
    auto hub=open(root/L"bin/neonmix-hub.exe",false),worker=open(root/L"airplay/bin/neonmix-airplay-worker.exe",false);
    auto marker=open(root/L"neonmix-installation.json",false);
    // Compiled hashes cannot be replaced by a writable user manifest after UAC.
    if(digest(hub->value)!=NEONMIX_HUB_SHA256||digest(worker->value)!=NEONMIX_WORKER_SHA256)throw std::runtime_error("network_package_hash_mismatch");
    const auto record=root/L"network-instance.ini",journal=root/L"network-journal.ini";
    std::wstring user=sid(GetCurrentProcess());
    std::unique_ptr<Handle> caller;
    if(requester){
        caller=std::make_unique<Handle>(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION|SYNCHRONIZE,FALSE,requester));
        if(!caller->value||WaitForSingleObject(caller->value,0)!=WAIT_TIMEOUT)throw std::runtime_error("network_requester_unavailable");
        std::vector<wchar_t> path(32768);DWORD length=static_cast<DWORD>(path.size());
        if(!QueryFullProcessImageNameW(caller->value,0,path.data(),&length))throw std::runtime_error("network_requester_invalid");
        auto caller_image=open(fs::path(std::wstring(path.data(),length)),false),self=open(module,false);
        FILETIME original{},current{},exit{},kernel{},cpu{};
        if(!same_file(caller_image->value,self->value)||!GetProcessTimes(caller->value,&original,&exit,&kernel,&cpu)||
           !GetProcessTimes(GetCurrentProcess(),&current,&exit,&kernel,&cpu)||CompareFileTime(&original,&current)>=0)
            throw std::runtime_error("network_requester_invalid");
        user=sid(caller->value);
    }
    const bool exists=fs::exists(record);
    if(!exists&&(operation!=L"install"||requester))throw std::runtime_error("network_record_missing");
    ConfigFile installation(record,!requester&&operation!=L"inspect",user);
    if(!exists){
        installation.set(L"Instance",guid());installation.set(L"OwnerSid",user);installation.set(L"Root",root.wstring());installation.set(L"ControlPort",std::to_wstring(port?port:7443));
    }
    auto instance=installation.get(L"Instance");
    if(!valid_guid(instance)||installation.get(L"OwnerSid")!=user||_wcsicmp(installation.get(L"Root").c_str(),root.c_str())!=0)throw std::runtime_error("network_record_invalid");
    auto configured_port=installation.get(L"ControlPort");
    if(configured_port.empty()||configured_port.find_first_not_of(L"0123456789")!=std::wstring::npos)throw std::runtime_error("network_record_invalid");
    unsigned long saved_port=std::stoul(configured_port);
    if(saved_port<1||saved_port>65535)throw std::runtime_error("network_record_invalid");
    if(!port)port=static_cast<unsigned>(saved_port);
    if(!requester&&operation!=L"inspect"&&operation!=L"remove")installation.set(L"ControlPort",std::to_wstring(port));
    const std::wstring group=L"NeonMix."+instance;
    const std::wstring attribution=L"; owner="+user+L"; root="+installation.get(L"Root");
    if(operation!=L"inspect"&&!elevated()){
        // Only this same pinned helper is relaunched. The original owner process
        // remains alive, and the elevated child verifies its handle/image/SID.
        SHELLEXECUTEINFOW execute{};execute.cbSize=sizeof(execute);execute.fMask=SEE_MASK_NOCLOSEPROCESS|SEE_MASK_NOASYNC;
        std::wstring args=operation+L" --port "+std::to_wstring(port)+L" --request-pid "+std::to_wstring(GetCurrentProcessId());
        execute.lpVerb=L"runas";execute.lpFile=module.c_str();execute.lpParameters=args.c_str();execute.nShow=SW_HIDE;
        if(!ShellExecuteExW(&execute)){std::cout<<"{\"configuration_code\":4,\"effective_policy_code\":4,\"network_ready\":false,\"uac_cancelled\":true}\n";return 20;}
        Handle child(execute.hProcess);WaitForSingleObject(child.value,INFINITE);DWORD code=1;if(!GetExitCodeProcess(child.value,&code))return 20;return static_cast<int>(code);
    }
    // Record is immutable during privileged reads; it is closed before journal
    // writes. The pinned ancestor handles keep the path bound to this instance.
    check(CoInitializeEx(nullptr,COINIT_APARTMENTTHREADED));struct Uninitialize{~Uninitialize(){CoUninitialize();}}uninitialize;
    Com<INetFwPolicy2> policy;check(CoCreateInstance(__uuidof(NetFwPolicy2),nullptr,CLSCTX_INPROC_SERVER,__uuidof(INetFwPolicy2),reinterpret_cast<void**>(&policy.value)));
    Com<INetFwRules> rules;check(policy->get_Rules(&rules.value));auto expected=templates(root,port);
    if(operation==L"inspect"){report(inspect(policy.value,rules.value,expected,group));return 0;}
    std::array<Com<INetFwRule>,4> previous;
    // Verify attribution before any mutations. A rule with a colliding name but
    // a foreign group/owner is not adopted or deleted.
    for(size_t i=0;i<4;i++){
        Text name(group+L"."+expected[i].suffix);HRESULT hr=rules->Item(name.value,&previous[i].value);
        if(FAILED(hr)){if(hr!=HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND))check(hr);continue;}
        if(read_text(previous[i].value,&INetFwRule::get_Grouping)!=group||
           !([](const std::wstring& text,const std::wstring& tail){return text.size()>=tail.size()&&text.compare(text.size()-tail.size(),tail.size(),tail)==0;})(read_text(previous[i].value,&INetFwRule::get_Description),attribution))
            throw std::runtime_error("network_rule_owner_mismatch");
    }
    ConfigFile changes(journal,true,user);
    changes.set(L"Instance",instance);changes.set(L"Operation",operation);changes.set(L"Status",L"pending");
    std::vector<size_t> changed;
    try {
        for(size_t i=0;i<4;i++){
            Text name(group+L"."+expected[i].suffix);
            if(operation!=L"remove"&&previous[i].value&&matches(previous[i].value,expected[i],group))continue;
            changes.set(L"AttemptedRule",expected[i].suffix);changed.push_back(i);
            if(previous[i].value)check(rules->Remove(name.value));
            if(operation!=L"remove"){
                Com<INetFwRule> rule;check(CoCreateInstance(__uuidof(NetFwRule),nullptr,CLSCTX_INPROC_SERVER,__uuidof(INetFwRule),reinterpret_cast<void**>(&rule.value)));
                fill(rule.value,expected[i],group+L"."+expected[i].suffix,group,attribution);check(rules->Add(rule.value));
                Com<INetFwRule> readback;check(rules->Item(name.value,&readback.value));
                if(!matches(readback.value,expected[i],group))throw std::runtime_error("network_rule_readback_failed");
            }else{Com<INetFwRule> readback;if(SUCCEEDED(rules->Item(name.value,&readback.value)))throw std::runtime_error("network_rule_remove_failed");}
            changes.set(L"CompletedRule",expected[i].suffix);
        }
        changes.set(L"Status",L"complete");
    }catch(...){
        bool restored=true;
        for(auto it=changed.rbegin();it!=changed.rend();++it){Text name(group+L"."+expected[*it].suffix);rules->Remove(name.value);if(previous[*it].value&&FAILED(rules->Add(previous[*it].value)))restored=false;}
        changes.set(L"Status",restored?L"rolled_back":L"rollback_failed");throw;
    }
    auto status=inspect(policy.value,rules.value,expected,group);report(status);
    return operation==L"remove"?0:status.configuration!=0||status.blocked?20:0;
}
int wmain(int argc,wchar_t** argv){
    try {
        if(argc<2)throw std::runtime_error("network_operation_required");
        std::wstring operation=argv[1];if(operation!=L"install"&&operation!=L"repair"&&operation!=L"remove"&&operation!=L"inspect")throw std::runtime_error("network_operation_invalid");
        unsigned port=0;DWORD requester=0;
        for(int i=2;i<argc;i+=2){
            if(i+1==argc)throw std::runtime_error("network_argument_invalid");
            std::wstring value=argv[i+1];if(value.empty()||value.find_first_not_of(L"0123456789")!=std::wstring::npos)throw std::runtime_error("network_argument_invalid");
            unsigned long number=std::stoul(value);
            if(std::wstring(argv[i])==L"--port"&&number>=1&&number<=65535)port=static_cast<unsigned>(number);
            else if(std::wstring(argv[i])==L"--request-pid"&&number>0)requester=static_cast<DWORD>(number);
            else throw std::runtime_error("network_argument_invalid");
        }
        return run(operation,port,requester);
    }catch(const std::exception& error){std::cout<<"{\"configuration\":\"unknown\",\"configuration_code\":4,\"effective_policy_code\":4,\"network_ready\":false,\"error\":\""<<([](const char* raw){std::string value(raw);return value.rfind("network_",0)==0&&value.size()<80&&value.find_first_not_of("abcdefghijklmnopqrstuvwxyz_")==std::string::npos?value:std::string("network_api_failed");})(error.what())<<"\"}\n";return 20;}
}
