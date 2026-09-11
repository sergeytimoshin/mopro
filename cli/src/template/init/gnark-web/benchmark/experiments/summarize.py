#!/usr/bin/env python3
"""Summarize sessions without pooling away process-to-process variation."""
import argparse,json,statistics,gzip,hashlib
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('root',type=Path);a=p.parse_args();r=a.root.resolve()
records=[]
for path in sorted((r/'reports').glob('*.json')):
 d=json.loads(path.read_text())
 if 'proveMs' not in d:continue
 assert not d.get('error'),path
 phases=[json.loads(proof['profile']) for proof in d['proofs'][:len(d['proveMs'])]]
 records.append({'name':path.stem,'stage':path.stem.split('-')[0],**{k:d[k] for k in ['backend','threads','pools','fixture','startupMs','prepareMs','firstProofMs','proveMs','medianProveMs','artifacts','fixtureHashes','nativeVerifiedProofs','recordedAtUnix']},'phaseMs':phases})
med=statistics.median
summary={'sessions':records,'nativeVerifiedProofs':sum(x['nativeVerifiedProofs'] for x in records),'bench':{},'keys':{},'workers':{},'assets':{}}
variants=['arkworks','montgomery','ffjavascript','ark-glv','ark-serial']
for variant in variants:
 summary['bench'][variant]={}
 for fixture in ['square','mimc','commitments']:
  rows=[x for x in records if x['name'].startswith('bench-'+fixture+'-raw-'+variant+'-16-')]
  if not rows:continue
  summary['bench'][variant][fixture]={'sessions':len(rows),'medianMs':med(x['medianProveMs'] for x in rows),'sessionMedians':[x['medianProveMs'] for x in rows],'startupMs':med(x['startupMs'] for x in rows),'prepareMs':med(x['prepareMs'] for x in rows),'firstProofMs':med(x['firstProofMs'] for x in rows),'phases':{key:med(med(d[key] for d in x['phaseMs']) for x in rows) for key in rows[0]['phaseMs'][0]}}
 g=r/variant/'web/MoproWasmBindings/gnark'
 assets={}
 for path in sorted(g.rglob('*')):
  if path.is_file() and path.suffix in ['.wasm','.js']:
   data=path.read_bytes();assets[str(path.relative_to(g))]={'bytes':len(data),'gzipBytes':len(gzip.compress(data,mtime=0)),'sha256':hashlib.sha256(data).hexdigest()}
 summary['assets'][variant]=assets
for fixture in ['square','mimc','commitments']:
 summary['keys'][fixture]={}
 for raw in [False,True]:
  label=fixture+('-raw' if raw else '')
  rows=[x for x in records if x['name'].startswith('keys-'+label+'-arkworks-16-')]
  if rows:summary['keys'][fixture]['raw' if raw else 'compressed']={'sessions':len(rows),'prepareMs':med(x['prepareMs'] for x in rows),'pkDecodeMs':med(med(p['preparePk'] for p in x['phaseMs']) for x in rows),'firstProofMs':med(x['firstProofMs'] for x in rows)}
  directory=(r/'arkworks/web/assets'/label).resolve()
  sizes={}
  for name in ['circuit.pk','circuit.vk','circuit.r1cs']:
   data=(directory/name).read_bytes();sizes[name]={'bytes':len(data),'gzipBytes':len(gzip.compress(data,mtime=0))}
  summary['keys'][fixture].setdefault('raw' if raw else 'compressed',{})['files']=sizes
 summary['workers'][fixture]={str(x['threads']):x['medianProveMs'] for x in records if x['stage']=='workers' and x['name'].startswith('workers-'+fixture+'-raw-')}
(r/'summary.json').write_text(json.dumps(summary,indent=2))
print(json.dumps({k:v for k,v in summary.items() if k in ['bench','keys','workers','nativeVerifiedProofs']},indent=2))
