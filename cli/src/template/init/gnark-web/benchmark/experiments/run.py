#!/usr/bin/env python3
"""Run one browser at a time; independently verify every recorded proof."""
import argparse, hashlib, json, os, subprocess, time
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('root',type=Path);p.add_argument('stage',choices=['checks','bench','workers','keys','portable']);p.add_argument('--verifier',type=Path,required=True);p.add_argument('--port',type=int,default=3217);p.add_argument('--threads',type=int,nargs='+',choices=range(1,65),default=[1,2,4,8,16],help='worker sweep counts');p.add_argument('--sessions',type=int,default=1,help='worker sweep sessions');p.add_argument('--samples',type=int,default=9,help='worker sweep timed samples');p.add_argument('--variants',nargs='+',choices=['arkworks','montgomery','ffjavascript','ark-glv','ark-serial']);a=p.parse_args()
r=a.root.resolve();h=Path(__file__).resolve().parent;reports=r/'reports';reports.mkdir(exist_ok=True)
base=dict(os.environ,CHROME_BIN=os.environ.get('CHROME_BIN','/home/ubuntu/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome'),CHROMEDRIVER_BIN=os.environ.get('CHROMEDRIVER_BIN','/home/ubuntu/.cache/selenium/chromedriver/linux64/153.0.8010.36/chromedriver'))
def hashes(g):
 return {str(p.relative_to(g)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(g.rglob('*')) if p.is_file() and p.suffix in ['.wasm','.js']}
def run(variant,fixture,threads,session,count=31,mode="rust"):
 name=f'{a.stage}-{fixture}-{variant}-{threads}-{session}';report=reports/(name+'.json');web=r/variant/'web';g=web/'MoproWasmBindings/gnark'
 env=dict(base,MOPRO_GNARK_MODE=mode,MOPRO_GNARK_THREADS=str(threads),MOPRO_GNARK_BENCH_BACKEND=variant,MOPRO_GNARK_BENCH_SAMPLES=str(count),MOPRO_GNARK_BENCH_WARMUPS='5',MOPRO_GNARK_BENCH_URL=f'http://127.0.0.1:{a.port}/{variant}/web/gnark-benchmark.html',MOPRO_GNARK_BENCH_BASE=f'./assets/{fixture}/',MOPRO_GNARK_BENCH_REPORT=str(report))
 print('Starting',name,flush=True);started=time.time();before=hashes(g)
 with(reports/(name+'.log')).open('w')as log:subprocess.run(['node',str(h/'browser.cjs')],cwd=web,env=env,stdout=log,stderr=subprocess.STDOUT,check=True,timeout=360)
 result=json.loads(report.read_text())
 if mode=='portable':
  assert result['execution']=={'arithmetic':'go','solver':'go'}
  assert result['threads']==0
  result.update(backend='go',pools={},requestedBackend=variant)
 else:
  assert result['backend']==variant;assert result['threads']==threads
 fixture_dir=(web/'assets'/fixture).resolve();env['MOPRO_GNARK_BENCH_FIXTURES']=str(fixture_dir)
 with(reports/(name+'-native.log')).open('w')as log:subprocess.run([str(a.verifier),'-test.run=^TestBrowserBenchmarkProofs$'],env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
 assert before==hashes(g),'artifact changed during benchmark'
 result['artifacts']=before;result['fixtureHashes']={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in fixture_dir.iterdir() if p.is_file()};result['recordedAtUnix']=started;result['nativeVerifiedProofs']=len(result['proofs']);report.write_text(json.dumps(result,indent=2))
 print(f"{name}: prove {result['medianProveMs']:.2f} ms; prepare {result['prepareMs']:.1f} ms; {len(result['proofs'])} native verified",flush=True)
if a.stage=='checks':
 for variant in ['montgomery','ffjavascript','ark-glv','ark-serial']:
  for threads in [1,16]:
   name=f'check-{variant}-{threads}';env=dict(base,MOPRO_GNARK_THREADS=str(threads),MOPRO_GNARK_BENCH_URL=f'http://127.0.0.1:{a.port}/{variant}/web/gnark-benchmark.html',MOPRO_GNARK_BENCH_REPORT=str(reports/(name+'.json')))
   print('Starting',name,flush=True)
   with(reports/(name+'.log')).open('w')as log:subprocess.run(['node',str(h/'check.cjs')],cwd=r/variant/'web',env=env,stdout=log,stderr=subprocess.STDOUT,check=True,timeout=360)
   print(name,json.loads((reports/(name+'.json')).read_text()),flush=True)
elif a.stage=='bench':
 variants=a.variants or ['arkworks','montgomery','ffjavascript','ark-glv','ark-serial']
 for session in range(1,4):
  order=variants[session-1:]+variants[:session-1]
  if session==2:order=order[::-1]
  for fixture in ['square-raw','mimc-raw','commitments-raw']:
   for variant in order:run(variant,fixture,16,session)
elif a.stage=='workers':
 for fixture in ['square-raw','mimc-raw','commitments-raw']:
  for threads in a.threads:
   for session in range(1,a.sessions+1):run('arkworks',fixture,threads,session,a.samples)
elif a.stage=='portable':
 for variant in ['arkworks','montgomery','ffjavascript']:run(variant,'hints-raw',2,1,9,'portable')
else:
 for session in range(1,4):
  for fixture in ['square','mimc','commitments']:
   for name in ([fixture,fixture+'-raw'] if session%2 else [fixture+'-raw',fixture]):run('arkworks',name,16,session,9)
print(a.stage,'completed',flush=True)
