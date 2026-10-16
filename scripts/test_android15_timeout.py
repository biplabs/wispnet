#!/usr/bin/env python3
"""Verify Android 15 framework dataSync timeout on an isolated emulator test app.

Requires x86_64 peercheck APK/test APK installed on a disposable Android 15
emulator. Never changes the physical tablet or the normal Wispnet app.
"""
import argparse
import subprocess
from relay_smoke import run

PKG='com.biplabs.wispnet.peercheck'
def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--serial',required=True);args=parser.parse_args()
    assert args.serial.startswith('emulator-'),'Timeout override is restricted to a disposable emulator'
    def adb(*cmd,**kwargs):return run(['adb','-s',args.serial,*cmd],**kwargs)
    assert int(adb('shell','getprop','ro.build.version.sdk').stdout.strip())>=35,'Requires Android 15+'
    original=adb('shell','device_config','get','activity_manager','data_sync_fgs_timeout_duration').stdout.strip()
    try:
        adb('shell','am','force-stop',PKG)
        adb('shell','pm','grant',PKG,'android.permission.POST_NOTIFICATIONS')
        adb('shell','device_config','put','activity_manager','data_sync_fgs_timeout_duration','10000')
        adb('shell','am','compat','enable','FGS_INTRODUCE_TIME_LIMITS',PKG)
        result=adb('shell','am','instrument','-w','-e','mode','platform-timeout',PKG+'.test/com.biplabs.wispnet.AppTestRunner',timeout=120)
        print(result.stdout)
        assert 'framework-timeout-stopped-native-access' in result.stdout and 'result=passed' in result.stdout,'Android 15 timeout acceptance failed'
        print('PASS: actual Android 15 framework timeout stopped native access; explicit foreground restart and Stop succeeded')
    finally:
        adb('shell','am','force-stop',PKG,ok=None)
        adb('shell','am','compat','reset','FGS_INTRODUCE_TIME_LIMITS',PKG,ok=None)
        if original=='null':adb('shell','device_config','delete','activity_manager','data_sync_fgs_timeout_duration',ok=None)
        else:adb('shell','device_config','put','activity_manager','data_sync_fgs_timeout_duration',original,ok=None)

if __name__=='__main__':main()
