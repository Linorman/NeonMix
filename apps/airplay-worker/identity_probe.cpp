/* SPDX-License-Identifier: GPL-3.0-or-later; test-only identity loader probe. */
#include <stdio.h>
#include <stdbool.h>
#include "platform.h"
extern "C" {
#include "crypto.h"
#include "pairing.h"
}

static std::filesystem::path replacement;
static void replace_validated_path(const std::string& path) {
    std::error_code error;
    std::filesystem::rename(replacement,std::filesystem::u8path(path),error);
#ifdef _WIN32
    if(!error)throw std::runtime_error("replacement_unexpectedly_succeeded");
#else
    if(error)throw std::runtime_error("replacement_fixture_failed");
#endif
}
static int probe(const std::string &path) {
    try {
    auto pem=platform::read_private_key(path,replacement.empty()?nullptr:replace_validated_path);
    pairing_t *pairing = pairing_init_from_pem(pem->bytes.data(), pem->size);
    if (!pairing) return 1;
    unsigned char public_key[ED25519_KEY_SIZE];
    pairing_get_public_key(pairing, public_key);
    pairing_destroy(pairing);
    ed25519_key_t *key = ed25519_key_from_pem(pem->bytes.data(), pem->size);
    pem->clear();
    if (!key) return 1;
    unsigned char signature[64];
    ed25519_sign(signature, sizeof(signature), (const unsigned char *) "", 0, key);
    ed25519_key_destroy(key);
    for (size_t i = 0; i < sizeof(public_key); i++) printf("%02x", public_key[i]);
    putchar('\n');
    for (size_t i = 0; i < sizeof(signature); i++) printf("%02x", signature[i]);
    putchar('\n');
    return 0;
    } catch(const std::exception &error) { fprintf(stderr,"%s\n",error.what()); return 1; }
}
#ifdef _WIN32
static std::string utf8(const wchar_t *value) {
    int size=WideCharToMultiByte(CP_UTF8,WC_ERR_INVALID_CHARS,value,-1,nullptr,0,nullptr,nullptr);
    if(!size)throw std::runtime_error("invalid_path");
    std::string path(size,'\0');
    if(!WideCharToMultiByte(CP_UTF8,WC_ERR_INVALID_CHARS,value,-1,path.data(),size,nullptr,nullptr))throw std::runtime_error("invalid_path");
    path.pop_back();return path;
}
int wmain(int argc, wchar_t **argv) {
    if(argc==2 && std::wstring(argv[1])==L"--path-roots") {
        using V=std::vector<std::wstring>;
        const std::pair<std::wstring,V> cases[]={
            {L"E:\\key.pem",{L"E:\\"}},
            {L"E:\\state\\key.pem",{L"E:\\",L"E:\\state"}},
            {L"\\\\?\\E:\\state\\key.pem",{L"\\\\?\\E:\\",L"\\\\?\\E:\\state"}},
            {L"\\\\server\\share\\state\\key.pem",{L"\\\\server\\share\\",L"\\\\server\\share\\state"}},
            {L"\\\\?\\UNC\\server\\share\\state\\key.pem",{L"\\\\?\\UNC\\server\\share\\",L"\\\\?\\UNC\\server\\share\\state"}}
        };
        for(const auto &test:cases)if(platform::private_key_parents(test.first)!=test.second)return 1;
        for(const auto &path:{L"\\\\?\\",L"relative\\key.pem",L"\\\\server\\",L"\\\\?\\UNC\\server\\"}) {
            try{platform::private_key_parents(path);return 1;}catch(const std::runtime_error&){}
        }
        puts("windows_path_roots_ok");return 0;
    }
    if(argc!=2&&argc!=3)return 2;
    if(argc==3)replacement=std::filesystem::path(argv[2]);
    return probe(utf8(argv[1]));
}
#else
int main(int argc, char **argv) {
    if(argc!=2&&argc!=3)return 2;
    if(argc==3)replacement=std::filesystem::path(argv[2]);
    return probe(argv[1]);
}
#endif
