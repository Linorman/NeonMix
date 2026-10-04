#!/usr/bin/env python3
"""Prepare pinned, audio-only UxPlay worker. Invoke through tools/dev or tools/dev.ps1.
All downloads, SDK extraction, source patches and build output stay in .local/airplay.
"""
import argparse
import concurrent.futures
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / '.local/airplay'
COMMIT = 'df67c212a433cf6dda3676dd40c097900d24e645'
SOURCES = {
 'uxplay-v1.73.7.tar.gz': ('https://codeload.github.com/FDH2/UxPlay/tar.gz/' + COMMIT, '168de53ed688f87c4022d79357ce91f9608ad5c53236bff95134d19b88efeb3f'),
 'libplist-2.6.0.tar.gz': ('https://codeload.github.com/libimobiledevice/libplist/tar.gz/refs/tags/2.6.0', 'e6491c2fa3370e556ac41b8705dd7f8f0e772c8f78641c3878cabd45bd84d950'),
 'gstreamer-devel-1.28.7.pkg': ('https://gstreamer.freedesktop.org/data/pkg/osx/1.28.7/gstreamer-1.0-devel-1.28.7-universal.pkg', '72a44870cf02472cbf6e9a84bcc25ee6807dd1c26659a112066373544d365e7a'),
}

def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda: f.read(8*1024*1024), b''): h.update(block)
    return h.hexdigest()

def fetch(name, url, expected):
    target = BASE/'downloads'/name
    if target.exists() and digest(target) == expected: return target
    if name.endswith('.pkg'):
        size = 753854434
        chunk = 8*1024*1024
        def part(index):
            start=index*chunk; end=min(size,start+chunk)-1
            path=target.parent/f'gst-devel-{index:03}.part'
            if path.exists() and path.stat().st_size==end-start+1: return path
            for attempt in range(5):
                try:
                    req=urllib.request.Request(url+f'?neonmix-airplay={index}',headers={'Range':f'bytes={start}-{end}'})
                    with urllib.request.urlopen(req,timeout=60) as response: data=response.read()
                    if len(data)!=end-start+1: raise ValueError('incorrect download range')
                    path.write_bytes(data); return path
                except Exception:
                    if attempt==4: raise
        with concurrent.futures.ThreadPoolExecutor(max_workers=10) as pool:
            parts=list(pool.map(part,range((size+chunk-1)//chunk)))
        with target.open('wb') as out:
            for path in parts: out.write(path.read_bytes()); path.unlink()
    else:
        with urllib.request.urlopen(url,timeout=60) as response, target.open('wb') as out:
            shutil.copyfileobj(response,out)
    if digest(target)!=expected: raise RuntimeError(f'checksum mismatch: {name}')
    return target

def project_path(value):
    path=Path(value).resolve()
    if not path.is_relative_to(ROOT):
        raise argparse.ArgumentTypeError('AirPlay development paths must stay inside the project')
    return path

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sources-only',action='store_true',help='Prepare only checksum-locked UxPlay/libplist and the maintained patch')
    parser.add_argument('--gst-prefix',type=project_path,help='Already extracted project-local GStreamer runtime/development prefix')
    parser.add_argument('--sdk-prefix',type=project_path,help='Separate project-local development headers (macOS default: .local/airplay/sdk)')
    parser.add_argument('--build-dir',type=project_path,default=BASE/'build',help='Project-local CMake build directory')
    parser.add_argument('--cmake-arg',action='append',default=[],help='Additional CMake configuration argument; use --cmake-arg=-D...')
    parser.add_argument('--no-copy',action='store_true',help='Keep build artifacts in build-dir without replacing Hub-side worker copies')
    args=parser.parse_args()
    tmpdir=Path(os.environ.get('TMPDIR') or os.environ.get('TMP') or os.environ.get('TEMP') or '/').resolve()
    if not tmpdir.is_relative_to(ROOT):
        raise SystemExit('Run through tools/dev (POSIX) or tools/dev.ps1 (Windows)')
    for sub in ['downloads','src','sdk','build','plugins']: (BASE/sub).mkdir(parents=True,exist_ok=True)
    selected={name:record for name,record in SOURCES.items() if not name.endswith('.pkg')}
    if sys.platform=='darwin' and not args.sources_only and not args.sdk_prefix:
        selected['gstreamer-devel-1.28.7.pkg']=SOURCES['gstreamer-devel-1.28.7.pkg']
    archives={name:fetch(name,*record) for name,record in selected.items()}
    for name in ['uxplay-v1.73.7.tar.gz','libplist-2.6.0.tar.gz']:
        with tarfile.open(archives[name]) as tar: tar.extractall(BASE/'src',filter='data')
    pristine=BASE/'src'/('UxPlay-'+COMMIT); patched=BASE/'patched'
    if patched.exists(): shutil.rmtree(patched)
    shutil.copytree(pristine,patched)
    subprocess.run(['patch','-p1','--input',str(ROOT/'patches/airplay-audio-only.patch')],cwd=patched,check=True)
    if args.sources_only:
        print(patched)
        return
    if sys.platform not in ['darwin','linux','win32']:
        raise SystemExit('Supported build hosts: macOS, Linux, Windows (MinGW pthread toolchain).')
    runtime=args.gst_prefix or (ROOT/'.local/gstreamer/prefix' if sys.platform=='darwin' else None)
    if not runtime or not runtime.is_dir():
        raise SystemExit('Supply --gst-prefix with an already extracted project-local GStreamer SDK/runtime. macOS default requires tools/prepare_gstreamer.py first.')
    sdk=args.sdk_prefix or (BASE/'sdk' if sys.platform=='darwin' else runtime)
    marker=BASE/'sdk/.extracted'
    if sys.platform=='darwin' and not args.sdk_prefix and (not marker.exists() or marker.read_text()!=SOURCES['gstreamer-devel-1.28.7.pkg'][1]):
        expanded=BASE/'gst-devel-expanded'
        if not expanded.exists(): subprocess.run(['pkgutil','--expand-full',str(archives['gstreamer-devel-1.28.7.pkg']),str(expanded)],check=True)
        for payload in expanded.glob('*.pkg/Payload'):
            for sub in ['include','lib/glib-2.0/include']:
                source=payload/sub
                if source.exists(): shutil.copytree(source,BASE/'sdk'/sub,dirs_exist_ok=True)
        if not (BASE/'sdk/include/gstreamer-1.0/gst/gst.h').exists(): raise RuntimeError('development SDK headers missing')
        marker.write_text(SOURCES['gstreamer-devel-1.28.7.pkg'][1])
        shutil.rmtree(expanded)
    # config.h is generated by CMake for the target, never hard-coded from the host.
    (BASE/'src/libplist-2.6.0/config.h').unlink(missing_ok=True)
    # The worker gets its own audio-only plugin registry, without changing native media.
    suffix='.dylib' if sys.platform=='darwin' else '.dll' if sys.platform=='win32' else '.so'
    for stale in (BASE/'plugins').iterdir():
        if stale.is_file() or stale.is_symlink(): stale.unlink()
    for name in ['coreelements','app','audioconvert','audioresample','libav']:
        candidates=list(runtime.rglob(f'*gst{name}{suffix}'))
        if len(candidates)!=1: raise RuntimeError(f'Expected one audio plugin gst{name}{suffix}, found {candidates}')
        link=BASE/'plugins'/candidates[0].name
        if sys.platform=='win32': shutil.copy2(candidates[0],link)
        else: link.symlink_to(candidates[0])
    command=['cmake','-S',str(ROOT/'apps/airplay-worker'),'-B',str(args.build_dir),'-DCMAKE_BUILD_TYPE=Release',
             f'-DNEONMIX_GSTREAMER_PREFIX={runtime.as_posix()}',f'-DNEONMIX_GSTREAMER_SDK={sdk.as_posix()}']
    if sys.platform=='darwin': command.append('-DCMAKE_OSX_DEPLOYMENT_TARGET=14.6')
    if sys.platform=='win32': command.extend(['-G','Ninja'])
    command.extend(args.cmake_arg)
    subprocess.run(command,check=True)
    subprocess.run(['cmake','--build',str(args.build_dir),'--parallel','8'],check=True)
    worker=args.build_dir/('neonmix-airplay-worker.exe' if sys.platform=='win32' else 'neonmix-airplay-worker')
    if not args.no_copy:
        for profile in ['debug','release']:
            destination=ROOT/'target'/profile/worker.name
            destination.parent.mkdir(parents=True,exist_ok=True)
            shutil.copy2(worker,destination)
    print(worker)

if __name__=='__main__': main()
