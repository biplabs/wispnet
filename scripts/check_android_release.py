#!/usr/bin/env python3
"""Validate the actual release bundle, native alignment and optional upload signature."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import urllib.request
import xml.etree.ElementTree as ET
import zipfile

ROOT=Path(__file__).resolve().parents[1]
VERSION='1.18.3'
SHA256='a099cfa1543f55593bc2ed16a70a7c67fe54b1747bb7301f37fdfd6d91028e29'
ANDROID='{http://schemas.android.com/apk/res/android}'


def bundletool(*arguments):
    jar=ROOT/'target/android-tools'/f'bundletool-all-{VERSION}.jar'
    if not jar.exists():
        jar.parent.mkdir(parents=True,exist_ok=True)
        with urllib.request.urlopen(f'https://github.com/google/bundletool/releases/download/{VERSION}/{jar.name}',timeout=60) as response:
            data=response.read(128*1024*1024)
        if hashlib.sha256(data).hexdigest()!=SHA256:raise ValueError('bundletool download checksum mismatch')
        temporary=jar.with_suffix('.download');temporary.write_bytes(data);temporary.replace(jar)
    if hashlib.sha256(jar.read_bytes()).hexdigest()!=SHA256:raise ValueError('bundletool cache checksum mismatch')
    java=Path(os.environ.get('JAVA_HOME','/opt/android-studio/jbr'))/'bin/java'
    return subprocess.run([java,'-jar',jar,*map(str,arguments)],capture_output=True,text=True,check=True).stdout


def check_elf(data,abi,require_relro=True):
    if data[:6]!=b'\x7fELF\x02\x01':raise ValueError('Expected little-endian ELF64')
    expected={'arm64-v8a':183,'x86_64':62}[abi]
    if struct.unpack_from('<H',data,18)[0]!=expected:raise ValueError('Native ABI mismatch')
    start=struct.unpack_from('<Q',data,32)[0];size,count=struct.unpack_from('<HH',data,54)
    if size!=56 or not count or start+size*count>len(data):raise ValueError('Invalid ELF program headers')
    loads=[];relros=[]
    for i in range(count):
        kind,flags,offset,address,physical,file_size,memory_size,alignment=struct.unpack_from('<IIQQQQQQ',data,start+i*size)
        if kind==1:
            loads.append((address,memory_size,flags))
            if alignment<16384 or alignment&(alignment-1) or offset%16384!=address%16384:raise ValueError('Native LOAD segment is not 16 KB aligned')
        if kind==0x6474e552:
            relros.append((address,memory_size))
    if not loads or (require_relro and not relros):raise ValueError('Native LOAD/RELRO segments missing')
    for address,memory_size in relros:
        end=address+memory_size
        if end%16384==0:continue
        # Android also permits a complete, separate RELRO LOAD segment when
        # rounding protection up cannot cover subsequent writable data:
        # https://developer.android.com/guide/practices/page-sizes#relro
        own_segment=any(a==address and size==memory_size for a,size,flags in loads)
        protected_start=address//16384*16384;protected_end=(end+16383)//16384*16384
        overlap=any(flags&2 and (a,size)!=(address,memory_size) and a<protected_end and a+size>protected_start for a,size,flags in loads)
        if not own_segment or overlap:raise ValueError('Native RELRO boundary is not 16 KB aligned')


def check_manifest(text):
    manifest=ET.fromstring(text);app=manifest.find('application');sdk=manifest.find('uses-sdk')
    if manifest.get('package')!='app.wispnet.android':raise ValueError('Unexpected release application ID')
    if int(manifest.get(ANDROID+'versionCode','0'))<=0:raise ValueError('Invalid version code')
    if app is None or app.get(ANDROID+'debuggable','false')!='false' or app.get(ANDROID+'allowBackup')!='false':raise ValueError('Release must be non-debuggable with backup disabled')
    if sdk is None or int(sdk.get(ANDROID+'targetSdkVersion','0'))<36:raise ValueError('Release target must be API 36+')
    for feature in manifest.findall('uses-feature'):
        if feature.get(ANDROID+'name','').startswith('android.hardware.camera') and feature.get(ANDROID+'required','true')!='false':raise ValueError('Camera must remain optional')
    return manifest.get(ANDROID+'versionCode')


def check_libraries(libraries,abi):
    native={f'base/lib/{abi}/libwispnet_android.so'}
    flutter=native|{f'base/lib/{abi}/libflutter.so',f'base/lib/{abi}/libapp.so'}
    terminal=flutter|{f'base/lib/{abi}/libtermux.so',f'base/lib/{abi}/libandroidx.graphics.path.so'}
    drop={f'base/lib/{abi}/libdesktop_drop_ffi.so'}
    if set(libraries) not in (native,flutter,terminal,flutter|drop,terminal|drop) or len(libraries)!=len(set(libraries)):
        raise ValueError('Unexpected native bundle contents')


def check(path,abi='arm64-v8a',require_signed=False):
    bundletool('validate','--bundle',path)
    version=check_manifest(bundletool('dump','manifest','--bundle',path,'--module','base'))
    config=json.loads(bundletool('dump','config','--bundle',path))
    native=config['optimizations']['uncompressNativeLibraries']
    if not native.get('enabled') or native.get('alignment')!='PAGE_ALIGNMENT_16K':raise ValueError('Bundle must request 16 KB native ZIP alignment')
    with zipfile.ZipFile(path) as archive:
        libraries=[name for name in archive.namelist() if name.endswith('.so')]
        check_libraries(libraries,abi)
        for name in libraries:
            # Dart's generated AOT snapshot has no GNU_RELRO segment. Its LOAD
            # alignment is still checked; Rust and the engine must retain RELRO.
            check_elf(archive.read(name),abi,require_relro=not name.endswith('/libapp.so'))
        signed=any(name.startswith('META-INF/') and name.endswith(('.RSA','.EC','.DSA')) for name in archive.namelist())
    if require_signed and not signed:raise ValueError('Upload-signed bundle required; refusing unsigned artifact')
    if signed:
        signer=Path(os.environ.get('JAVA_HOME','/opt/android-studio/jbr'))/'bin/jarsigner'
        result=subprocess.run([signer,'-J-Duser.language=en','-J-Duser.country=US','-verify',str(path)],capture_output=True,text=True,check=True)
        if 'jar verified.' not in result.stdout:raise ValueError('Bundle signature verification failed')
    print(f'PASS: API 36+ release bundle version {version}, {abi}, 16 KB ELF/RELRO and ZIP configuration; '+('signature verified' if signed else 'UNSIGNED — signing required before upload'))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('bundle',type=Path);parser.add_argument('--abi',choices=['arm64-v8a','x86_64'],default='arm64-v8a');parser.add_argument('--require-signed',action='store_true');args=parser.parse_args()
    check(args.bundle,args.abi,args.require_signed)
