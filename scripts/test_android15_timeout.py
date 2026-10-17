#!/usr/bin/env python3
"""Verify Android 15 framework dataSync timeout on an isolated emulator test app.

Requires matching x86_64 APK/test APKs installed on a disposable Android 15+
emulator. Never changes the physical tablet or the normal Wispnet app.
"""
import argparse
import subprocess
from relay_smoke import run

PKG='app.wispnet.android.peercheck'
def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--serial',required=True);parser.add_argument('--package',choices=['app.wispnet.android.peercheck','app.wispnet.android'],default=PKG);args=parser.parse_args();package=args.package
    assert args.serial.startswith('emulator-'),'Timeout override is restricted to a disposable emulator'
    def adb(*cmd,**kwargs):return run(['adb','-s',args.serial,*cmd],**kwargs)
    assert int(adb('shell','getprop','ro.build.version.sdk').stdout.strip())>=35,'Requires Android 15+'
    original=adb('shell','device_config','get','activity_manager','data_sync_fgs_timeout_duration').stdout.strip()
    try:
        adb('shell','am','force-stop',package)
        adb('shell','pm','grant',package,'android.permission.POST_NOTIFICATIONS')
        adb('shell','device_config','put','activity_manager','data_sync_fgs_timeout_duration','10000')
        adb('shell','am','compat','enable','FGS_INTRODUCE_TIME_LIMITS',package)
        result=adb('shell','am','instrument','-w','-e','mode','platform-timeout',package+'.test/com.biplabs.wispnet.AppTestRunner',timeout=120)
        print(result.stdout)
        assert 'framework-timeout-stopped-native-access' in result.stdout and 'result=passed' in result.stdout,'Framework timeout acceptance failed'
        print('PASS: actual Android framework timeout stopped native access; explicit foreground restart and Stop succeeded')
    finally:
        adb('shell','am','force-stop',package,ok=None)
        adb('shell','am','compat','reset','FGS_INTRODUCE_TIME_LIMITS',package,ok=None)
        if original=='null':adb('shell','device_config','delete','activity_manager','data_sync_fgs_timeout_duration',ok=None)
        else:adb('shell','device_config','put','activity_manager','data_sync_fgs_timeout_duration',original,ok=None)

if __name__=='__main__':main()
