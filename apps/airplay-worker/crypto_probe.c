/* SPDX-License-Identifier: GPL-3.0-or-later; test-only primitive wrappers. */
#include <openssl/evp.h>
#include <openssl/rand.h>
#include "crypto.h"
int probe_ed_public(unsigned char*priv,unsigned char*pub){EVP_PKEY*k=EVP_PKEY_new_raw_private_key(EVP_PKEY_ED25519,NULL,priv,32);size_t n=32;int ok=EVP_PKEY_get_raw_public_key(k,pub,&n);EVP_PKEY_free(k);return ok;}
int probe_sign(unsigned char*priv,unsigned char*msg,unsigned char*sig){EVP_PKEY*k=EVP_PKEY_new_raw_private_key(EVP_PKEY_ED25519,NULL,priv,32);EVP_MD_CTX*c=EVP_MD_CTX_new();size_t n=64;int ok=EVP_DigestSignInit(c,NULL,NULL,NULL,k)&&EVP_DigestSign(c,sig,&n,msg,64);EVP_MD_CTX_free(c);EVP_PKEY_free(k);return ok;}
int probe_verify(unsigned char*pub,unsigned char*msg,unsigned char*sig){EVP_PKEY*k=EVP_PKEY_new_raw_public_key(EVP_PKEY_ED25519,NULL,pub,32);EVP_MD_CTX*c=EVP_MD_CTX_new();int ok=EVP_DigestVerifyInit(c,NULL,NULL,NULL,k)&&EVP_DigestVerify(c,sig,64,msg,64);EVP_MD_CTX_free(c);EVP_PKEY_free(k);return ok;}
int probe_x_public(unsigned char*priv,unsigned char*pub){EVP_PKEY*k=EVP_PKEY_new_raw_private_key(EVP_PKEY_X25519,NULL,priv,32);size_t n=32;int ok=EVP_PKEY_get_raw_public_key(k,pub,&n);EVP_PKEY_free(k);return ok;}
int probe_x_secret(unsigned char*priv,unsigned char*pub,unsigned char*secret){EVP_PKEY*k=EVP_PKEY_new_raw_private_key(EVP_PKEY_X25519,NULL,priv,32);EVP_PKEY*p=EVP_PKEY_new_raw_public_key(EVP_PKEY_X25519,NULL,pub,32);EVP_PKEY_CTX*c=EVP_PKEY_CTX_new(k,NULL);size_t n=32;int ok=EVP_PKEY_derive_init(c)&&EVP_PKEY_derive_set_peer(c,p)&&EVP_PKEY_derive(c,secret,&n);EVP_PKEY_CTX_free(c);EVP_PKEY_free(k);EVP_PKEY_free(p);return ok;}
int probe_gcm(unsigned char*msg,int n,unsigned char*key,unsigned char*iv,unsigned char*out,unsigned char*tag){return gcm_encrypt(msg,n,out,key,iv,tag);}
int probe_gcm_decrypt(unsigned char*msg,int n,unsigned char*key,unsigned char*iv,unsigned char*out,unsigned char*tag){return gcm_decrypt(msg,n,out,key,iv,tag);}
void probe_ctr(unsigned char*msg,int n,unsigned char*key,unsigned char*iv,unsigned char*out){aes_ctx_t*c=aes_ctr_init(key,iv);aes_ctr_encrypt(c,msg,out,n);aes_ctr_destroy(c);}

#include "playfair/playfair.h"
void probe_fairplay(unsigned char*msg,unsigned char*encrypted,unsigned char*out){playfair_decrypt(msg,encrypted,out);}
void probe_cbc(unsigned char*msg,int n,unsigned char*key,unsigned char*iv,unsigned char*out){aes_ctx_t*c=aes_cbc_init(key,iv,AES_ENCRYPT);aes_cbc_encrypt(c,msg,out,n);aes_cbc_destroy(c);}
