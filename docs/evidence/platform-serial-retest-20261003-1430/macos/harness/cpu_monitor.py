import ctypes,json,time,datetime,signal,sys,os
lib=ctypes.CDLL('/usr/lib/libSystem.B.dylib');lib.mach_host_self.restype=ctypes.c_uint
cores=os.cpu_count();running=True
def stop(*a):
 global running
 running=False
signal.signal(signal.SIGTERM,stop)
def ticks():
 data=(ctypes.c_uint*4)();count=ctypes.c_uint(4)
 result=lib.host_statistics(lib.mach_host_self(),3,ctypes.byref(data),ctypes.byref(count))
 if result:raise RuntimeError(result)
 return list(data)
prev=ticks()
with open(sys.argv[1],'a',buffering=1) as f:
 while running:
  time.sleep(1);current=ticks();d=[(x-y)%2**32 for x,y in zip(current,prev)];prev=current;t=sum(d);u=100*(t-d[2])/t if t else 0
  f.write(json.dumps(dict(timestamp=datetime.datetime.now(datetime.timezone(datetime.timedelta(hours=8))).isoformat(),epoch=time.time(),util=round(u,3),logical_cores=cores))+'\n')
