/* SPDX-License-Identifier: GPL-3.0-or-later
 * TXT serialization only. Hub is the sole discovery owner; never calls DNS-SD.
 */
#include "dnssd.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
struct dnssd_s {char *name;char hw[6];char pk[65];char receiver_uuid[37];uint64_t features;char air[2048],raop[2048];int air_len,raop_len;};
static void txt(char *buf,int *len,const char *key,const char*value){size_t n=strlen(key)+1+strlen(value);if(n>255||*len+n+1>2048)abort();buf[(*len)++]=(char)n;memcpy(buf+*len,key,strlen(key));*len+=strlen(key);buf[(*len)++]='=';memcpy(buf+*len,value,strlen(value));*len+=strlen(value);}
static void build(dnssd_t*d){char feature[32],id[18];snprintf(feature,sizeof(feature),"0x%X,0x%X",(unsigned)d->features,(unsigned)(d->features>>32));snprintf(id,sizeof(id),"%02X:%02X:%02X:%02X:%02X:%02X",(unsigned char)d->hw[0],(unsigned char)d->hw[1],(unsigned char)d->hw[2],(unsigned char)d->hw[3],(unsigned char)d->hw[4],(unsigned char)d->hw[5]);d->air_len=d->raop_len=0;
#define AIR(k,v) txt(d->air,&d->air_len,k,v)
#define RAOP(k,v) txt(d->raop,&d->raop_len,k,v)
AIR("deviceid",id);AIR("pi",d->receiver_uuid);AIR("features",feature);AIR("model","AppleTV3,2");AIR("pk",d->pk);AIR("pw","true");AIR("flags","0x4");AIR("srcvers","220.68");AIR("vv","2");
RAOP("model","AppleTV3,2");RAOP("srcvers","220.68");RAOP("da","true");RAOP("sv","false");RAOP("pi",d->receiver_uuid);RAOP("txtvers","1");RAOP("ch","2");RAOP("cn","0,1,2,3");RAOP("et","0,3,5");RAOP("sr","44100");RAOP("ss","16");RAOP("tp","UDP");RAOP("md","0");RAOP("pw","true");RAOP("sf","0x4");RAOP("vs","220.68");RAOP("am","AppleTV3,2");RAOP("ft",feature);RAOP("pk",d->pk);RAOP("vn","65537");RAOP("vv","2");
}
dnssd_t*dnssd_init(const char*n,int nl,const char*hw,int hl,int*error,unsigned char pw){if(error)*error=0;if(hl!=6||nl<1||nl>80)return NULL;dnssd_t*d=calloc(1,sizeof(*d));if(!d)return NULL;d->name=calloc(1,nl+1);memcpy(d->name,n,nl);memcpy(d->hw,hw,6);return d;}
const char*dnssd_get_raop_txt(dnssd_t*d,int*l){build(d);*l=d->raop_len;return d->raop;}
const char*dnssd_get_airplay_txt(dnssd_t*d,int*l){build(d);*l=d->air_len;return d->air;}
const char*dnssd_get_name(dnssd_t*d,int*l){*l=strlen(d->name);return d->name;}
const char*dnssd_get_hw_addr(dnssd_t*d,int*l){*l=6;return d->hw;}
void dnssd_set_receiver_uuid(dnssd_t*d,const char*uuid){snprintf(d->receiver_uuid,sizeof(d->receiver_uuid),"%s",uuid);}
void dnssd_set_pk(dnssd_t*d,char*p){snprintf(d->pk,sizeof(d->pk),"%s",p);}
void dnssd_set_airplay_features(dnssd_t*d,int b,int v){if(b<0||b>63)return;uint64_t mask=(uint64_t)1<<b;if(v)d->features|=mask;else d->features&=~mask;}
uint64_t dnssd_get_airplay_features(dnssd_t*d){return d->features;}
void dnssd_destroy(dnssd_t*d){if(d){free(d->name);free(d);}}
int dnssd_register_raop(dnssd_t*d,unsigned short p){return -1;}
int dnssd_register_airplay(dnssd_t*d,unsigned short p){return -1;}
void dnssd_unregister_raop(dnssd_t*d){}
void dnssd_unregister_airplay(dnssd_t*d){}
