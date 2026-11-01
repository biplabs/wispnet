import io
import os
import zipfile
from pathlib import Path
import struct
import subprocess
import unittest
import xml.etree.ElementTree as ET
from check_android_release import check_elf,check_manifest,check_libraries,check_jni_callbacks

class ReleaseChecks(unittest.TestCase):
    def elf(self,align=16384,relro_end=16384,machine=183):
        data=bytearray(64+112);data[:6]=b'\x7fELF\x02\x01';struct.pack_into('<H',data,18,machine);struct.pack_into('<Q',data,32,64);struct.pack_into('<HH',data,54,56,2)
        struct.pack_into('<IIQQQQQQ',data,64,1,5,0,0,0,64,64,align)
        struct.pack_into('<IIQQQQQQ',data,120,0x6474e552,4,0,0,0,64,relro_end,1)
        return data
    def test_load_alignment_is_not_enough_without_relro_alignment(self):
        check_elf(self.elf(),'arm64-v8a')
        for data in [self.elf(4096),self.elf(relro_end=4096),self.elf(machine=62),self.elf()[:80]]:
            with self.assertRaises(ValueError):check_elf(data,'arm64-v8a')
    def test_dart_snapshot_can_omit_relro_but_not_load_alignment(self):
        snapshot=self.elf();struct.pack_into('<I',snapshot,120,0)
        check_elf(snapshot,'arm64-v8a',require_relro=False)
        with self.assertRaises(ValueError):check_elf(snapshot,'arm64-v8a')
        struct.pack_into('<Q',snapshot,64+48,4096)
        with self.assertRaises(ValueError):check_elf(snapshot,'arm64-v8a',require_relro=False)
    def test_separate_relro_load_cannot_overlap_a_writable_page(self):
        data=bytearray(64+3*56);data[:6]=b'\x7fELF\x02\x01'
        struct.pack_into('<H',data,18,183);struct.pack_into('<Q',data,32,64);struct.pack_into('<HH',data,54,56,3)
        struct.pack_into('<IIQQQQQQ',data,64,1,6,0x680,0x4680,0,0x1d8,0x980,16384)
        struct.pack_into('<IIQQQQQQ',data,120,0x6474e552,4,0x680,0x4680,0,0x1d8,0x980,1)
        struct.pack_into('<IIQQQQQQ',data,176,1,6,0,0x8000,0,0x100,0x1000,16384)
        check_elf(data,'arm64-v8a')
        overlapping=bytearray(data)
        struct.pack_into('<Q',overlapping,176+8,0x2000);struct.pack_into('<Q',overlapping,176+16,0x6000)
        with self.assertRaises(ValueError):check_elf(overlapping,'arm64-v8a')
        struct.pack_into('<Q',data,64+40,0x1980)
        with self.assertRaises(ValueError):check_elf(data,'arm64-v8a')
    def test_manifest_rejects_debug_backup_old_target_and_required_camera(self):
        manifest='<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="app.wispnet.android" android:versionCode="2"><uses-sdk android:targetSdkVersion="36"/><uses-feature android:name="android.hardware.camera.any" android:required="false"/><application android:debuggable="false" android:allowBackup="false"/></manifest>'
        self.assertEqual(check_manifest(manifest),'2')
        for invalid in [manifest.replace('targetSdkVersion="36"','targetSdkVersion="35"'),manifest.replace('debuggable="false"','debuggable="true"'),manifest.replace('allowBackup="false"','allowBackup="true"'),manifest.replace('required="false"','required="true"')]:
            with self.assertRaises(ValueError):check_manifest(invalid)
    def test_flutter_release_requires_engine_app_and_rust_for_the_selected_abi(self):
        native=['base/lib/arm64-v8a/libwispnet_android.so']
        flutter=native+['base/lib/arm64-v8a/libflutter.so','base/lib/arm64-v8a/libapp.so']
        check_libraries(native,'arm64-v8a')
        check_libraries(flutter,'arm64-v8a')
        terminal=flutter+['base/lib/arm64-v8a/libtermux.so','base/lib/arm64-v8a/libandroidx.graphics.path.so']
        check_libraries(terminal,'arm64-v8a')
        drop=['base/lib/arm64-v8a/libdesktop_drop_ffi.so']
        check_libraries(flutter+drop,'arm64-v8a')
        check_libraries(terminal+drop,'arm64-v8a')
        for invalid in [native+drop,terminal+drop+drop,terminal+drop+['base/lib/arm64-v8a/extra.so']]:
            with self.assertRaises(ValueError):check_libraries(invalid,'arm64-v8a')
        for invalid in [terminal[:-1],terminal+['base/lib/arm64-v8a/extra.so']]:
            with self.assertRaises(ValueError):check_libraries(invalid,'arm64-v8a')
        for invalid in [flutter[:-1],flutter+['base/lib/arm64-v8a/unexpected.so'],flutter+native,[p.replace('arm64-v8a','x86_64') for p in flutter]]:
            with self.assertRaises(ValueError):check_libraries(invalid,'arm64-v8a')
    def test_private_state_excluded_from_cloud_and_device_transfer(self):
        root=Path(__file__).resolve().parents[1]/'apps/flutter/android/app/src/main'
        android='{http://schemas.android.com/apk/res/android}'
        app=ET.parse(root/'AndroidManifest.xml').getroot().find('application')
        self.assertEqual(app.get(android+'allowBackup'),'false')
        self.assertEqual(app.get(android+'dataExtractionRules'),'@xml/data_extraction_rules')
        rules=ET.parse(root/'res/xml/data_extraction_rules.xml').getroot()
        for mode in ['cloud-backup','device-transfer']:
            section=rules.find(mode)
            self.assertIsNotNone(section)
            self.assertEqual(section.findall('include'),[])
            exclusions={(item.get('domain'),item.get('path')) for item in section.findall('exclude')}
            for domain in ['root','file','database','sharedpref','external','device_root','device_file','device_database','device_sharedpref']:
                self.assertIn((domain,'.'),exclusions)
    def test_signed_build_fails_before_building_when_credentials_missing(self):
        env=os.environ.copy()
        for name in ['WISPNET_UPLOAD_KEYSTORE','WISPNET_UPLOAD_STORE_PASSWORD','WISPNET_UPLOAD_KEY_ALIAS','WISPNET_UPLOAD_KEY_PASSWORD']:env.pop(name,None)
        result=subprocess.run([str(Path(__file__).resolve().parent/'build_android_release.sh'),'--signed'],env=env,capture_output=True,text=True,timeout=5)
        self.assertNotEqual(result.returncode,0);self.assertIn('requires all four',result.stderr)


class JniReleaseChecks(unittest.TestCase):
    def dex(self, omitted=(), renamed=False, static=True, body=True, signature=True):
        # Small DEX table fixture. References stay present when definitions are
        # omitted, reproducing the difference between strings and callable ABI.
        names = ['call', 'publicationCancelled', 'publishReceivedFile', 'putFileDescriptor', 'shareSourceRequest', 'shareSourceCancelled']
        strings = ['Lcom/biplabs/wispnet/Native;', 'Ljava/lang/String;', 'I', 'Z', '[B', *names]
        if renamed: strings[7] = 'renamedByR8'
        data = bytearray(112 + len(strings)*4 + 5*4 + 4*12 + 6*8 + 32)
        data[:8] = b'dex\n035\0'
        strings_off = 112; types_off = strings_off + len(strings)*4
        protos_off = types_off + 20; methods_off = protos_off + 48; classes_off = methods_off + 48
        for at, size, offset in [(56,len(strings),strings_off),(64,5,types_off),(72,4,protos_off),(88,6,methods_off),(96,1,classes_off)]:
            struct.pack_into('<II',data,at,size,offset)
        for index, text in enumerate(strings):
            struct.pack_into('<I',data,strings_off+index*4,len(data))
            data.extend(bytes([len(text)]) + text.encode() + b'\0')
        for index in range(5): struct.pack_into('<I',data,types_off+index*4,index)
        for index, (result, parameters) in enumerate([(1,[1]),(3,[1]),(1,[1,2]),(4,[1])]):
            while len(data)%4: data.append(0)
            offset = len(data); data.extend(struct.pack('<I',len(parameters)))
            for parameter in parameters: data.extend(struct.pack('<H',parameter))
            struct.pack_into('<III',data,protos_off+index*12,0,3 if index==2 and not signature else result,offset)
        for index, proto in enumerate([0,1,2,2,3,1]): struct.pack_into('<HHI',data,methods_off+index*8,0,proto,5+index)
        struct.pack_into('<I',data,classes_off+24,len(data))
        definitions = [i for i,name in enumerate(names) if name not in omitted]
        data.extend(bytes([0,0,len(definitions),0])); previous = 0
        def uleb(value):
            encoded=bytearray()
            while value>=128: encoded.append((value&127)|128); value>>=7
            encoded.append(value); return encoded
        for index in definitions:
            java = index in (2,4)
            flags = (8 if static or not java else 0) | (0 if java else 256)
            data.extend(uleb(index-previous)+uleb(flags)+uleb(1 if java and body else 0)); previous=index
        return bytes(data)

    def verify(self, data, entry='base/dex/classes2.dex'):
        stream=io.BytesIO()
        with zipfile.ZipFile(stream,'w') as archive: archive.writestr(entry,data)
        with zipfile.ZipFile(stream) as archive: check_jni_callbacks(archive)

    def test_callable_jni_survives_in_actual_apk_and_bundle_dex(self):
        for entry in ['classes.dex','classes2.dex','base/dex/classes.dex','base/dex/classes12.dex']:
            self.verify(self.dex(),entry)

    def test_removed_callbacks_fail_even_when_method_references_remain(self):
        for missing in [('publishReceivedFile',),('publicationCancelled',),('call',),('putFileDescriptor',),('shareSourceRequest',),('shareSourceCancelled',)]:
            with self.subTest(missing=missing), self.assertRaisesRegex(ValueError,missing[0]):
                self.verify(self.dex(omitted=missing))

    def test_renamed_nonstatic_or_bodyless_callback_is_not_callable(self):
        for broken in [self.dex(renamed=True),self.dex(static=False),self.dex(body=False),self.dex(signature=False)]:
            with self.assertRaisesRegex(ValueError,'publishReceivedFile'): self.verify(broken)

    def test_dex_in_assets_cannot_stand_in_for_installed_application_code(self):
        with self.assertRaisesRegex(ValueError,'publishReceivedFile'):
            self.verify(self.dex(),'assets/classes.dex')

if __name__=='__main__':unittest.main()
