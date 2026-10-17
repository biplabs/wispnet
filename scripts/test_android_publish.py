import tempfile
from pathlib import Path
import unittest
from publish_android_internal import BASE,MEDIA,publish

class Response:
    def __init__(self,value=None,status=200):self.value=value or {};self.status_code=status;self.ok=status<400;self.content=b'json'
    def json(self):return self.value

class Session:
    def __init__(self,fail_upload=False,wrong_verify=False):self.calls=[];self.edits=0;self.fail_upload=fail_upload;self.wrong_verify=wrong_verify
    def post(self,url,**kwargs):
        self.calls.append(('POST',url,kwargs.get('json')))
        if url==BASE:self.edits+=1;return Response({'id':str(self.edits)})
        if url.startswith(MEDIA):return Response({'error':{'message':'bad bundle'}},400) if self.fail_upload else Response({'versionCode':1})
        return Response({'id':'1'})
    def put(self,url,**kwargs):self.calls.append(('PUT',url,kwargs['json']));self.status=kwargs['json']['releases'][0]['status'];return Response(kwargs['json'])
    def get(self,url,**kwargs):self.calls.append(('GET',url,None));return Response({'releases':[{'versionCodes':['2' if self.wrong_verify else '1'],'status':getattr(self,'status','completed')}]})
    def delete(self,url,**kwargs):self.calls.append(('DELETE',url,None));return Response()

class PublishingTests(unittest.TestCase):
    def artifact(self,temporary):p=Path(temporary)/'test.aab';p.write_bytes(b'fixture');return p
    def test_publish_internal_only_then_verify_fresh_committed_state(self):
        with tempfile.TemporaryDirectory() as directory:
            session=Session();result=publish(self.artifact(directory),session)
        self.assertTrue(result['verified']);self.assertEqual(result['track'],'internal')
        update=next(call for call in session.calls if call[0]=='PUT');self.assertTrue(update[1].endswith('/tracks/internal'));self.assertEqual(update[2]['releases'][0]['versionCodes'],['1'])
        self.assertIn(('GET',BASE+'/2/tracks/internal',None),session.calls);self.assertIn(('DELETE',BASE+'/2',None),session.calls)
        self.assertLess(next(i for i,c in enumerate(session.calls) if c[1].endswith(':validate')),next(i for i,c in enumerate(session.calls) if c[1].endswith(':commit')))
    def test_upload_failure_discards_edit_without_track_update_or_commit(self):
        with tempfile.TemporaryDirectory() as directory:
            session=Session(fail_upload=True)
            with self.assertRaisesRegex(RuntimeError,'bad bundle'):publish(self.artifact(directory),session)
        self.assertIn(('DELETE',BASE+'/1',None),session.calls);self.assertFalse(any(c[0]=='PUT' or c[1].endswith(':commit') for c in session.calls))
    def test_post_commit_verification_failure_does_not_claim_success(self):
        with tempfile.TemporaryDirectory() as directory:
            session=Session(wrong_verify=True)
            with self.assertRaisesRegex(RuntimeError,'Commit succeeded but verification failed'):publish(self.artifact(directory),session)
        self.assertIn(('DELETE',BASE+'/2',None),session.calls);self.assertNotIn(('DELETE',BASE+'/1',None),session.calls)
    def test_draft_status_is_retained_for_bootstrap_apps(self):
        with tempfile.TemporaryDirectory() as directory:
            result=publish(self.artifact(directory),Session(),status='draft')
        self.assertEqual(result['status'],'draft');self.assertTrue(result['verified'])
    def test_commit_timeout_reports_unknown_outcome_without_retry(self):
        class TimeoutSession(Session):
            def post(self,url,**kwargs):
                if url.endswith(':commit'):self.calls.append(('POST',url,None));raise TimeoutError()
                return super().post(url,**kwargs)
        with tempfile.TemporaryDirectory() as directory:
            session=TimeoutSession()
            with self.assertRaisesRegex(RuntimeError,'Commit outcome unknown'):publish(self.artifact(directory),session)
        self.assertEqual(sum(c[1].endswith(':commit') for c in session.calls),1)
    def test_unsupported_status_has_no_side_effects(self):
        session=Session()
        with self.assertRaises(ValueError):publish(Path('unused'),session,status='inProgress')
        self.assertEqual(session.calls,[])

if __name__=='__main__':unittest.main()
