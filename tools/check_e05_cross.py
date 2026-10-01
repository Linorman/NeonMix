#!/usr/bin/env python3
"""macOS-hosted Windows/Linux E05 type checks; never run foreign executables.

Reuse installed pinned compilers read-only. Target libraries and headers can
only be extracted into .local/. Neither packages nor system libraries are installed.
"""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tomllib
from downloads import download

ROOT=Path(__file__).resolve().parents[1]
if sys.platform!='darwin':raise SystemExit('This cross-check entry runs on macOS only')
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--target',choices=['windows','linux','all'],default='all')
parser.add_argument('--clippy',action='store_true',help='Also lint the actual foreign cfg branches; still do not run them')
args=parser.parse_args()
CACHE=ROOT/'.local/e05-cross';CACHE.mkdir(parents=True,exist_ok=True)
OUT=ROOT/'artifacts/e05/review';OUT.mkdir(parents=True,exist_ok=True)
rustc=Path(subprocess.check_output(['rustup','which','--toolchain','1.95.0','rustc'],text=True).strip())
installed=rustc.parent.parent
manifest=tomllib.loads((installed/'lib/rustlib/multirust-channel-manifest.toml').read_text())
llvm=next((ROOT/'.local/cross-linux/llvm-tools').rglob('llvm-ar'),None)
if llvm is None:
    package=next(key for key in manifest['pkg'] if key.startswith('llvm-tools'))
    info=manifest['pkg'][package]['target']['aarch64-apple-darwin']
    archive=CACHE/Path(info['xz_url']).name
    download(info['xz_url'],archive,info['xz_hash'])
    with tarfile.open(archive) as data:data.extractall(CACHE,filter='data')
    llvm=next(CACHE.rglob('llvm-ar'))

def stdroot(target):
    if (installed/f'lib/rustlib/{target}/lib').is_dir():return installed
    existing=ROOT/'.local/cross-linux/rust'
    if (existing/f'lib/rustlib/{target}/lib').is_dir():return existing
    info=manifest['pkg']['rust-std']['target'][target]
    archive=CACHE/Path(info['xz_url']).name
    download(info['xz_url'],archive,info['xz_hash'])
    with tarfile.open(archive) as data:data.extractall(CACHE,filter='data')
    source=next(CACHE.glob(f'rust-std-*/rust-std-{target}/lib/rustlib/{target}'))
    directory=CACHE/'rust';destination=directory/f'lib/rustlib/{target}'
    destination.parent.mkdir(parents=True,exist_ok=True)
    if not destination.exists():destination.symlink_to(source,target_is_directory=True)
    return directory

def windows_headers():
    sysroot=CACHE/'mingw';sysroot.mkdir(exist_ok=True)
    version_file=sysroot/'dependency.json'
    if version_file.exists():return sysroot/'usr/share/mingw-w64/include',json.loads(version_file.read_text())
    for component in ('main','universe'):
        index=ROOT/f'.local/cross-linux/{component}-Packages.gz'
        download(f'https://archive.ubuntu.com/ubuntu/dists/noble/{component}/binary-amd64/Packages.gz',index)
        for paragraph in gzip.decompress(index.read_bytes()).decode().split('\n\n'):
            fields=dict(line.split(': ',1) for line in paragraph.splitlines() if ': ' in line and not line.startswith(' '))
            if fields.get('Package')!='mingw-w64-common':continue
            package=CACHE/Path(fields['Filename']).name
            download('https://archive.ubuntu.com/ubuntu/'+fields['Filename'],package,fields['SHA256'])
            members=subprocess.check_output(['ar','t',str(package)],text=True).splitlines()
            member=next(value for value in members if value.startswith('data.tar'))
            temporary=CACHE/member
            with temporary.open('wb') as output:subprocess.run(['ar','p',str(package),member],stdout=output,check=True)
            try:subprocess.run(['tar','-xf',str(temporary),'-C',str(sysroot)],check=True)
            finally:temporary.unlink(missing_ok=True)
            dependency={key:fields[key] for key in ('Package','Version','Filename','SHA256')}
            version_file.write_text(json.dumps(dependency,indent=2)+'\n')
            return sysroot/'usr/share/mingw-w64/include',dependency
    raise RuntimeError('Ubuntu index has no mingw-w64-common package')

report={'host':'macOS','foreign_programs_executed':False,'targets':{}}
for name in ('windows','linux'):
    if args.target not in ('all',name):continue
    target='x86_64-pc-windows-gnu' if name=='windows' else 'x86_64-unknown-linux-gnu'
    env=os.environ.copy();key=target.replace('-','_')
    env['CARGO_TARGET_'+key.upper()+'_RUSTFLAGS']=f'--sysroot={stdroot(target)}'
    env['CC_'+key]='clang';env['AR_'+key]=str(llvm)
    if name=='windows':
        include,dependency=windows_headers()
        env['CFLAGS_'+key]=f'--target=x86_64-w64-windows-gnu -I{include}'
    else:
        sysroot=ROOT/'.local/cross-linux/sysroot'
        if not (sysroot/'usr/include/x86_64-linux-gnu').is_dir():raise RuntimeError('Prepare project-local Linux headers with tools/cross_linux_check.py first')
        env['CFLAGS_'+key]=f'--target={target} --sysroot={sysroot} -I{sysroot}/usr/include/x86_64-linux-gnu'
        dependency={'sysroot':str(sysroot.relative_to(ROOT))}
    command=['cargo','clippy' if args.clippy else 'check','--locked','--all-targets','-p','neonmix-identity','-p','neonmix-control','--target',target]
    if args.clippy:command += ['--','-D','warnings']
    log=OUT/f"cross-{name}{'-clippy' if args.clippy else ''}.log"
    with log.open('w') as output:result=subprocess.run(command,cwd=ROOT,env=env,stdout=output,stderr=subprocess.STDOUT)
    report['targets'][name]={'target':target,'command':command,'exit_code':result.returncode,'header_dependency':dependency,'log':str(log.relative_to(ROOT))}
    print(name,result.returncode,flush=True)
    if result.returncode:
        print(log.read_text()[-5000:],flush=True)
        break
report['passed']=len(report['targets'])==(2 if args.target=='all' else 1) and all(item['exit_code']==0 for item in report['targets'].values())
report['source_sha256']={str(path.relative_to(ROOT)):hashlib.sha256(path.read_bytes()).hexdigest() for folder in ('crates/identity','crates/control') for path in (ROOT/folder).rglob('*') if path.is_file()}
(OUT/('cross-clippy.json' if args.clippy else 'cross-checks.json')).write_text(json.dumps(report,indent=2)+'\n')
raise SystemExit(0 if report['passed'] else 1)
