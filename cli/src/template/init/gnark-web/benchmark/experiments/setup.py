#!/usr/bin/env python3
"""Stage the exact same Go runtime, keys and browser harness for each backend."""
import argparse, shutil, os
from pathlib import Path
p=argparse.ArgumentParser(); p.add_argument('root',type=Path); p.add_argument('--fixtures',type=Path,required=True);p.add_argument('--selenium',type=Path,required=True); a=p.parse_args()
r=a.root.resolve();src=Path(__file__).resolve().parents[2]
for variant in ['arkworks','montgomery','ffjavascript','ark-glv','ark-serial']:
 web=r/variant/'web';g=web/'MoproWasmBindings/gnark';g.mkdir(parents=True,exist_ok=True)
 for name in ['gnark.js','gnark.worker.js','gnark.d.ts','LICENSE-APACHE']:shutil.copy2(src/name,g/name)
 shutil.copy2(src/'backends/rust.js',g/'gnark.backend.js')
 shutil.copy2(r/'gnark.wasm',g/'gnark.wasm')
 # Use the matching Go 1.24 standard runtime shim.
 import subprocess
 goroot=subprocess.check_output(['go','env','GOROOT'],env=dict(os.environ,GOTOOLCHAIN='go1.24.0'),text=True).strip()
 shutil.copy2(Path(goroot)/'lib/wasm/wasm_exec.js',g/'wasm_exec.js')
 shutil.copytree(r/'build/arkworks',g/'accelerator',dirs_exist_ok=True)
 (web/'gnark-benchmark.html').write_text('<!doctype html><meta charset="utf-8"><title>Gnark browser benchmark</title>')
 (web/'package.json').write_text('{"private":true,"dependencies":{"selenium-webdriver":"4.27.0"}}')
 if not (web/'node_modules').exists():(web/'node_modules').symlink_to(a.selenium.resolve(),target_is_directory=True)
 (web/'assets').mkdir(exist_ok=True)
 for fixture in a.fixtures.iterdir():
  if fixture.is_dir() and not (web/'assets'/fixture.name).exists():(web/'assets'/fixture.name).symlink_to(fixture.resolve(),target_is_directory=True)
 for fixture in (r/'fixtures').iterdir():
  if fixture.is_dir() and not (web/'assets'/fixture.name).exists():(web/'assets'/fixture.name).symlink_to(fixture.resolve(),target_is_directory=True)
