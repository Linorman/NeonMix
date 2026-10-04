"""Synthetic SRP/PIN and signed pair-verify sender for protocol seam tests."""
import ctypes, hashlib, os, plistlib, re
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
N=int('AC6BDB41324A9A9BF166DE5E1389582FAF72B6651987EE07FC3192943DB56050A37329CBB4A099ED8193E0757767A13DD52312AB4B03310DCD7F48A9DA04FD50E8083969EDB767B0CF6095179A163AB3661A05FBD5FAAAE82918A9962F0B93B855F97993EC975EEAA80D740ADBF4FF747359D041D5C33EA71D281E446B14773BCA97B43A23FB801676BD207A436C6481F1D2B9078717461A5B9D32E688F87748544523B524B0D57D5EA77A2775D2ECFA032CFBDBF52FB3786160279004E57AE6AF874E7303CE53299CCC041C7BC308D82A5698F3A8D0C38271AE35F8E9DBFBB694B5C803D89F7AE435DE236D525F54759B65E372FCD68EF20FA7111F9E4AFF73',16)
def bn(n):return n.to_bytes((n.bit_length()+7)//8,'big')
def h(s):return hashlib.sha1(s).digest()
def sha(s):return hashlib.sha512(s).digest()
class Crypto:
    def __init__(self):
        self.lib=ctypes.CDLL(str(ROOT/'.local/airplay/build/libneonmix-airplay-probe-crypto.dylib'))
        for n in ['probe_ed_public','probe_x_public','probe_sign','probe_x_secret']:
            f=getattr(self.lib,n);f.argtypes=[ctypes.c_void_p]*(3 if n in ['probe_sign','probe_x_secret'] else 2);f.restype=ctypes.c_int
        self.lib.probe_gcm.argtypes=[ctypes.c_void_p,ctypes.c_int,ctypes.c_void_p,ctypes.c_void_p,ctypes.c_void_p,ctypes.c_void_p];self.lib.probe_gcm.restype=ctypes.c_int
        self.lib.probe_gcm_decrypt.argtypes=self.lib.probe_gcm.argtypes;self.lib.probe_gcm_decrypt.restype=ctypes.c_int
        self.lib.probe_verify.argtypes=[ctypes.c_void_p]*3;self.lib.probe_verify.restype=ctypes.c_int
        for n in ['probe_ctr','probe_cbc']:
            f=getattr(self.lib,n);f.argtypes=[ctypes.c_void_p,ctypes.c_int,ctypes.c_void_p,ctypes.c_void_p,ctypes.c_void_p];f.restype=None
        self.lib.probe_fairplay.argtypes=[ctypes.c_void_p]*3;self.lib.probe_fairplay.restype=None
    def primitive(self,name,*args,size=32):
        out=ctypes.create_string_buffer(size);assert getattr(self.lib,name)(*args,out)==1;return out.raw
    def gcm(self,msg,key,iv):
        out=ctypes.create_string_buffer(len(msg));tag=ctypes.create_string_buffer(16);assert self.lib.probe_gcm(msg,len(msg),key,iv,out,tag)==len(msg);return out.raw,tag.raw
    def gcm_decrypt(self,msg,key,iv,tag):
        out=ctypes.create_string_buffer(len(msg));assert self.lib.probe_gcm_decrypt(msg,len(msg),key,iv,out,tag)==len(msg),'server pair-setup authentication failed';return out.raw
    def cipher(self,name,msg,key,iv):
        out=ctypes.create_string_buffer(len(msg));getattr(self.lib,name)(msg,len(msg),key,iv,out);return out.raw
    def fairplay(self,msg,encrypted):
        out=ctypes.create_string_buffer(16);self.lib.probe_fairplay(msg,encrypted,out);return out.raw

def pin_setup(request,crypto,pin,private,proof_bytes=20,expected_pin='1234',expected_server_public=None):
    user='11:22:33:44:55:66';root=lambda v:plistlib.dumps(v,fmt=plistlib.FMT_BINARY)
    code,body=request('POST','/pair-setup-pin',root(dict(method='pin',user=user)));assert code==200,(code,body)
    res=plistlib.loads(body);salt=res['salt'];B=int.from_bytes(res['pk'],'big');a=int.from_bytes(os.urandom(32),'big');A=pow(2,a,N)
    x=int.from_bytes(h(bn(int.from_bytes(salt,'big'))+h((user+':'+pin).encode())),'big')
    u=int.from_bytes(h(A.to_bytes(256,'big')+B.to_bytes(256,'big')),'big');k=int.from_bytes(h(N.to_bytes(256,'big')+(2).to_bytes(256,'big')),'big')
    S=pow((B-k*pow(2,x,N))%N,a+u*x,N);K=h(bn(S)+bytes(4))+h(bn(S)+b'\0\0\0\1')
    proof=h(bytes(a^b for a,b in zip(h(bn(N)),h(bn(2))))+h(user.encode())+bn(int.from_bytes(salt,'big'))+bn(A)+bn(B)+K)
    code,body=request('POST','/pair-setup-pin',root(dict(pk=bn(A),proof=proof+bytes(proof_bytes-20))))
    if pin!=expected_pin:assert code==470,(code,body);return None
    assert code==200,(code,body);assert plistlib.loads(body)['proof']==h(bn(A)+proof+K)
    pub=crypto.primitive('probe_ed_public',private);key=sha(b'Pair-Setup-AES-Key'+K)[:16];iv=bytearray(sha(b'Pair-Setup-AES-IV'+K)[:16]);iv[-1]=(iv[-1]+1)%256
    epk,tag=crypto.gcm(pub,key,bytes(iv));code,body=request('POST','/pair-setup-pin',root(dict(epk=epk,authTag=tag)));assert code==200,(code,body)
    if expected_server_public is not None:
        response=plistlib.loads(body);iv[-1]=(iv[-1]+1)%256
        assert crypto.gcm_decrypt(response['epk'],key,bytes(iv),response['authTag'])==expected_server_public,'receiver pair-setup identity changed'
    return pub

def pair_verify(request,crypto,private,pub,expected_server_public=None):
    xpriv=os.urandom(32);xpub=crypto.primitive('probe_x_public',xpriv)
    code,body=request('POST','/pair-verify',b'\1\0\0\0'+xpub+pub);assert code==200 and len(body)==96,(code,body)
    server=body[:32];secret=crypto.primitive('probe_x_secret',xpriv,server)
    sig=crypto.primitive('probe_sign',private,xpub+server,size=64)
    key=sha(b'Pair-Verify-AES-Key'+secret)[:16];iv=sha(b'Pair-Verify-AES-IV'+secret)[:16]
    if expected_server_public is not None:
        server_sig=crypto.cipher('probe_ctr',body[32:],key,iv)
        assert crypto.lib.probe_verify(expected_server_public,server+xpub,server_sig)==1,'receiver pair-verify identity changed'
    encrypted=crypto.cipher('probe_ctr',bytes(64)+sig,key,iv)[64:]
    code,body=request('POST','/pair-verify',bytes(4)+encrypted);assert code==200,(code,body)
    return secret
