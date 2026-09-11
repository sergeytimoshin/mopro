#!/usr/bin/env python3
"""Pack each generated runtime and test the actual installed tarball in Vite."""
import argparse,json,os,shutil,subprocess
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('root',type=Path);p.add_argument('--verifier',type=Path,required=True);a=p.parse_args();r=a.root.resolve();h=Path(__file__).resolve().parent
for variant in ['arkworks','montgomery','ffjavascript','ark-glv','ark-serial']:
 web=r/variant/'web';package=web/'MoproWasmBindings';archive=r/'packages'/variant;archive.mkdir(parents=True,exist_ok=True)
 report=json.loads(subprocess.check_output(['npm','pack','--json','--pack-destination',str(archive)],cwd=package,text=True));(archive/'pack.json').write_text(json.dumps(report,indent=2))
 files={entry['path'] for entry in report[0]['files']}
 assert all('gnark/'+name in files for name in ['gnark.js','gnark.worker.js','gnark.wasm','gnark.backend.js','wasm_exec.js'])
 if variant=='ffjavascript':
  assert 'gnark/licenses/ffjavascript-COPYING' in files
  assert not any(name.startswith('gnark/accelerator/') for name in files)
 else:
  assert 'gnark/accelerator/gnark_kernel_bg.wasm' in files
  assert any(name.startswith('gnark/accelerator/snippets/') for name in files)
 dest=r/'packed'/variant/'web';dest.mkdir(parents=True,exist_ok=True)
 subprocess.run(['tar','-xzf',str(archive/report[0]['filename']),'-C',str(dest)],check=True)
 # Only replace the generated installation of this experiment's tarball.
 if (dest/'MoproWasmBindings').exists():shutil.rmtree(dest/'MoproWasmBindings')
 (dest/'package').rename(dest/'MoproWasmBindings')
 for name in ['package.json','gnark-benchmark.html']:shutil.copy2(web/name,dest/name)
 if not (dest/'node_modules').exists():(dest/'node_modules').symlink_to((web/'node_modules').resolve(),target_is_directory=True)
 (dest/'assets').mkdir(exist_ok=True)
 if not (dest/'assets/gnark-solver').exists():(dest/'assets/gnark-solver').symlink_to(r/'fixtures/hints-raw',target_is_directory=True)
 env=dict(os.environ,MOPRO_GNARK_ACCELERATOR='true',MOPRO_GNARK_BENCH_BACKEND=variant)
 with(r/(variant+'-bundle.log')).open('w')as log:subprocess.run(['node',str(h/'bundler/test.mjs'),str(dest)],env=env,stdout=log,stderr=subprocess.STDOUT,check=True,timeout=180)
 for mode in ['go','rust']:
  env.update(MOPRO_GNARK_BENCH_REPORT=str(dest/f'gnark-vite-{mode}-benchmark.json'),MOPRO_GNARK_BENCH_FIXTURES=str(r/'fixtures/hints-raw'))
  with(r/(variant+'-bundle-'+mode+'-native.log')).open('w')as log:subprocess.run([str(a.verifier),'-test.run=^TestBrowserBenchmarkProofs$'],env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
 print(variant,'packed Vite, recovery and 18 native-verified proofs passed',flush=True)
