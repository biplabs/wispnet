#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["google-auth==2.40.3", "requests==2.32.3"]
# ///
"""Publish a verified upload-signed Wispnet bundle to internal testing only."""
import argparse
import json
import os
from pathlib import Path
from check_android_release import check

PACKAGE='com.biplabs.wispnet'
BASE=f'https://androidpublisher.googleapis.com/androidpublisher/v3/applications/{PACKAGE}/edits'
MEDIA=f'https://androidpublisher.googleapis.com/upload/androidpublisher/v3/applications/{PACKAGE}/edits'


def payload(response):
    if not response.ok:
        try:message=response.json().get('error',{}).get('message','Request rejected')
        except Exception:message='Request rejected'
        raise RuntimeError(f'Play Publisher HTTP {response.status_code}: {message}')
    return response.json() if response.content else {}


def publish(path,session,status='completed'):
    if status not in ('draft','completed'):raise ValueError('Only draft/completed internal releases are supported')
    edit=payload(session.post(BASE,json={},timeout=30))['id'];committed=False
    try:
        with path.open('rb') as stream:
            uploaded=payload(session.post(f'{MEDIA}/{edit}/bundles',params={'uploadType':'media'},data=stream,headers={'Content-Type':'application/octet-stream'},timeout=300))
        version=str(uploaded['versionCode'])
        release={'name':f'Wispnet {version}','versionCodes':[version],'status':status}
        payload(session.put(f'{BASE}/{edit}/tracks/internal',json={'track':'internal','releases':[release]},timeout=30))
        payload(session.post(f'{BASE}/{edit}:validate',json={},timeout=30))
        try:response=session.post(f'{BASE}/{edit}:commit',json={},timeout=60)
        except Exception as error:raise RuntimeError('Commit outcome unknown; inspect internal track before retrying') from error
        payload(response);committed=True
    finally:
        if not committed:session.delete(f'{BASE}/{edit}',timeout=30)
    # Verify committed state in a fresh transaction, not the edited preview.
    verify=None
    try:
        verify=payload(session.post(BASE,json={},timeout=30))['id']
        track=payload(session.get(f'{BASE}/{verify}/tracks/internal',timeout=30))
        if not any(version in r.get('versionCodes',[]) and r.get('status')==status for r in track.get('releases',[])):
            raise RuntimeError('Committed version/status not visible on internal track')
    except Exception as error:
        raise RuntimeError('Commit succeeded but verification failed; inspect internal track before retrying') from error
    finally:
        if verify:session.delete(f'{BASE}/{verify}',timeout=30)
    return {'package':PACKAGE,'track':'internal','versionCode':version,'status':status,'verified':True}


def authorized_session():
    from google.oauth2 import service_account
    from google.auth.transport.requests import AuthorizedSession
    scopes=['https://www.googleapis.com/auth/androidpublisher']
    if os.environ.get('ANDROID_PUBLISHER_CREDENTIALS'):
        credentials=service_account.Credentials.from_service_account_info(json.loads(os.environ['ANDROID_PUBLISHER_CREDENTIALS']),scopes=scopes)
    elif os.environ.get('GOOGLE_APPLICATION_CREDENTIALS'):
        credentials=service_account.Credentials.from_service_account_file(os.environ['GOOGLE_APPLICATION_CREDENTIALS'],scopes=scopes)
    else:raise ValueError('Set ANDROID_PUBLISHER_CREDENTIALS or GOOGLE_APPLICATION_CREDENTIALS to an authorized Play service account')
    return AuthorizedSession(credentials)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('bundle',type=Path);parser.add_argument('--publish',action='store_true');parser.add_argument('--release-status',choices=['draft','completed'],default='completed');args=parser.parse_args()
    check(args.bundle,require_signed=True)
    if not args.publish:
        print('DRY RUN: signed bundle validated; no credentials used or Play requests sent. Add --publish to upload and commit internal testing.');return
    with authorized_session() as session:print(json.dumps(publish(args.bundle,session,args.release_status)))


if __name__=='__main__':main()
