import os
from pathlib import Path
import struct
import subprocess
import unittest
import xml.etree.ElementTree as ET
from check_android_release import check_elf,check_manifest

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
    def test_manifest_rejects_debug_backup_old_target_and_required_camera(self):
        manifest='<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="app.wispnet.android" android:versionCode="2"><uses-sdk android:targetSdkVersion="36"/><uses-feature android:name="android.hardware.camera.any" android:required="false"/><application android:debuggable="false" android:allowBackup="false"/></manifest>'
        self.assertEqual(check_manifest(manifest),'2')
        for invalid in [manifest.replace('targetSdkVersion="36"','targetSdkVersion="35"'),manifest.replace('debuggable="false"','debuggable="true"'),manifest.replace('allowBackup="false"','allowBackup="true"'),manifest.replace('required="false"','required="true"')]:
            with self.assertRaises(ValueError):check_manifest(invalid)
    def test_private_state_excluded_from_cloud_and_device_transfer(self):
        root=Path(__file__).resolve().parents[1]/'apps/android/app/src/main'
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
        for name in ['WIPSNET_UPLOAD_KEYSTORE','WIPSNET_UPLOAD_STORE_PASSWORD','WIPSNET_UPLOAD_KEY_ALIAS','WIPSNET_UPLOAD_KEY_PASSWORD']:env.pop(name,None)
        result=subprocess.run([str(Path(__file__).resolve().parent/'build_android_release.sh'),'--signed'],env=env,capture_output=True,text=True,timeout=5)
        self.assertNotEqual(result.returncode,0);self.assertIn('requires all four',result.stderr)

if __name__=='__main__':unittest.main()
