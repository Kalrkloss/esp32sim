#!/usr/bin/env python3
"""Explain, but do not normalize away, C7 native layout/codegen differences."""
import collections, json, re, sys, types
from pathlib import Path
root, raw, output = (Path(s).resolve() for s in sys.argv[1:4])
m=types.ModuleType('asmcmp')
exec((root/'docs/evidence/classic-core/asmcmp.py').read_text().split('a, b = funcs(sys.argv[1])')[0],m.__dict__)
imm=re.compile(r'#-?0x[0-9a-f]+|#\d+')
rows=[]
for label in ['base','main']:
    d=raw/('codegen-'+label)
    a,b=m.funcs(d/'base.dis'),m.funcs(d/'candidate.dis')
    selected=json.loads((d/'codegen.json').read_text())['rows']
    for row in selected:
        name=row['name']
        if row['same']: continue
        if name.endswith('>::tick'):
            old,new=a[name],b[name]
            hist=lambda xs: collections.Counter(i.split()[0] for i in xs)
            h0,h1=hist(old),hist(new)
            delta={k:h1[k]-h0[k] for k in sorted(set(h0)|set(h1)) if h0[k]!=h1[k]}
            windows=[]
            for index,line in enumerate(new):
                if 'camera_dma_data' in line:
                    windows.append(dict(index=index,instructions=new[max(0,index-35):index+12]))
            rows.append(dict(comparison=label,name=name,classification='active-DMA camera dispatch plus layout/stack/register allocation changes',instruction_delta=len(new)-len(old),opcode_delta=delta,camera_call_windows=windows))
            assert windows
            continue
        assert len(a[name])==len(b[name]),name
        changes=[]
        for x,y in zip(a[name],b[name]):
            if x==y: continue
            if imm.sub('#IMM',x)==imm.sub('#IMM',y):
                xx=[int(i[1:],0) for i in imm.findall(x)]; yy=[int(i[1:],0) for i in imm.findall(y)]
                assert all(v-u in [0,16] for u,v in zip(xx,yy)),(name,x,y)
                changes.append(dict(before=x,after=y))
            else:
                mi,mj=m.TGT.search(x),m.TGT.search(y)
                assert mi and mj and m.TGT.sub('',x)==m.TGT.sub('',y) and m.shape(a,mi.group(1))==m.shape(b,mj.group(1)),(name,x,y)
        rows.append(dict(comparison=label,name=name,classification='same operations/registers/order; changed immediates only +16-byte struct field displacement (plus equivalent call aliases)',changed_field_operands=len(changes),examples=changes[:3]))
output.write_text(json.dumps(dict(method='Additional explanation only; original 10/21 equality score is not changed',rows=rows),indent=2)+'\n')
print('explained',len(rows),'function comparisons')
for row in rows:
    if 'camera_call_windows' in row: print(json.dumps(row,indent=2))
