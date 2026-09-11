#!/usr/bin/env python3
"""Build all comparison artifacts without changing a deployed demo."""
import argparse, json, os, shutil, subprocess
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('root',type=Path)
for name in ['montgomery','ffjavascript','tuning','fixtures','bindings','selenium']:p.add_argument('--'+name,type=Path,required=True)
p.add_argument('--wasm-pack',default='wasm-pack');a=p.parse_args();r=a.root.resolve();h=Path(__file__).resolve().parent;common=h.parents[1]
r.mkdir(parents=True,exist_ok=True)
def call(args, cwd=None, env=None):
 print(' '.join(map(str,args)),flush=True);subprocess.run(list(map(str,args)),cwd=cwd,env=env,check=True)
def source(repo):return repo.resolve()/'cli/src/template/init/gnark-web'
goenv=dict(os.environ,GOTOOLCHAIN='go1.24.0')
call(['go','build','-trimpath','-ldflags=-s -w','-o',r/'gnark.wasm','.'],common,dict(goenv,GOOS='js',GOARCH='wasm'))
call(['go','test','-c','-o',r/'verify.test','.'],common,goenv)
call(['go','run','./cmd/vectors','-out',r/'vectors'],common,goenv)
for fixture in ['square','mimc','commitments','hints']:
 call(['go','run','./cmd/reencode','-in',a.fixtures/fixture,'-out',r/'fixtures'/(fixture+'-raw')],common,goenv)
config='''[target.wasm32-unknown-unknown]
rustflags = [
 "-C", "target-feature=+atomics,+bulk-memory,+mutable-globals",
 "-C", "link-arg=--shared-memory", "-C", "link-arg=--import-memory",
 "-C", "link-arg=--max-memory=1073741824",
 "-C", "link-arg=--export=__wasm_init_tls", "-C", "link-arg=--export=__tls_size",
 "-C", "link-arg=--export=__tls_align", "-C", "link-arg=--export=__tls_base",
]
[unstable]
build-std = ["panic_abort", "std"]
'''
variants=[('arkworks',common,'bench-profile,hybrid'),('montgomery',source(a.montgomery),'hybrid-only'),('ark-glv',source(a.tuning),'bench-profile,hybrid,ark-glv'),('ark-serial',source(a.tuning),'bench-profile,hybrid,serial-msm')]
for name,src,features in variants:
 crate=r/'sources'/name;shutil.copytree(src/'accelerator',crate,dirs_exist_ok=True)
 shutil.copy2(crate/'Cargo.toml.template',crate/'Cargo.toml');(crate/'.cargo').mkdir(exist_ok=True);(crate/'.cargo/config.toml').write_text(config)
 call(['rustup','run','nightly-2025-11-15',a.wasm_pack,'build','--target','web','--release','--out-name','gnark_kernel','--out-dir',r/'build'/name,'--','--locked','--features',features],crate,dict(os.environ,CARGO_TARGET_DIR=str(r/'target')))
call(['python3',h/'setup.py',r,'--fixtures',a.fixtures,'--selenium',a.selenium])
deps=r/'deps';deps.mkdir(exist_ok=True)
for name in ['package.json','package-lock.json']:shutil.copy2(h/name,deps/name)
call(['npm','ci','--ignore-scripts'],deps)
for name,repo in [('montgomery',a.montgomery),('ffjavascript',a.ffjavascript)]:
 backend=source(repo)/'benchmark/experiments'/(name+'.js')
 library=deps/'node_modules'/name/('build/web/index.js' if name=='montgomery' else 'build/browser.esm.js')
 call(['node',deps/'node_modules/esbuild/bin/esbuild',backend,'--bundle','--platform=browser','--format=esm','--minify','--external:./accelerator/gnark_kernel.js','--alias:'+name+'='+str(library),'--outfile='+str(r/name/'web/MoproWasmBindings/gnark/gnark.backend.js')])
for name in ['arkworks','montgomery','ffjavascript','ark-glv','ark-serial']:
 package=r/name/'web/MoproWasmBindings';g=package/'gnark'
 for item in a.bindings.iterdir():
  if item.name=='gnark':continue
  if item.is_dir():shutil.copytree(item,package/item.name,dirs_exist_ok=True)
  else:shutil.copy2(item,package/item.name)
 if name!='ffjavascript':
  shutil.copytree(r/'build'/name,g/'accelerator',dirs_exist_ok=True)
  for license in ['LICENSE-APACHE','LICENSE-MIT']:shutil.copy2(common/'accelerator'/license,g/'accelerator'/license)
  (g/'accelerator/.npmignore').write_text('')
 else:shutil.rmtree(g/'accelerator')
 if name.startswith('ark-'):
  file=g/'gnark.backend.js';file.write_text(file.read_text().replace('backend: "arkworks"',f'backend: "{name}"'))
 licenses=g/'licenses';licenses.mkdir(exist_ok=True)
 dependencies={'montgomery':['montgomery','wasmati'],'ffjavascript':['ffjavascript','wasmcurves','wasmbuilder','web-worker']}.get(name,[])
 for dependency in dependencies:
  for filename in ['LICENSE','LICENSE.md','LICENSE.txt','COPYING','NOTICE']:
   src=deps/'node_modules'/dependency/filename
   if src.is_file():shutil.copy2(src,licenses/(dependency+'-'+filename))
print('Build complete. Start serve.py, then run.py checks/bench/workers/keys in sequence.')
