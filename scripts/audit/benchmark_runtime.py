#!/usr/bin/env python3
"""Release (-O) coordinate scan benchmark; extracts both kernels from the source.

Compares the same complete coordinate workload, without LocateV2 result budgets.
This measures coordinate scanning only, not end-to-end locate/serialization or GUI.
"""
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parents[2]
source = (root / 'crates/dozerd/src/file_mutation.rs').read_text()
reference = source[source.index('fn offset_to_line_col('):source.index('/// 单向游标:')]
cursor = source[source.index('struct LineCursor {'):source.index('/// 把 context 裁到预算内;')]
main = r'''
use std::{hint::black_box,time::Instant};
fn main(){
    println!("hits,variant,sample,ns,scanned_bytes,checksum");
    for n in [20_000usize,40_000,80_000] {
        let text="中x\n".repeat(n);
        let positions:Vec<_>=text.match_indices('x').map(|(p,_)|p).collect();
        for sample in 1..=5 {
            for variant in ["prefix","cursor"] {
                let mut sum=0u64;let mut cursor=LineCursor::new();let start=Instant::now();
                for &p in &positions {let (l,c)=if variant=="prefix"{offset_to_line_col(black_box(&text),black_box(p))} else {cursor.advance_to(black_box(&text),black_box(p));(cursor.line,cursor.col)};sum+=u64::from(l)+u64::from(c);}
                let elapsed=start.elapsed().as_nanos();black_box(sum);
                let scanned=if variant=="prefix"{positions.iter().sum::<usize>()}else{cursor.byte};
                println!("{n},{variant},{sample},{elapsed},{scanned},{sum}");
            }
        }
    }
}
'''
with tempfile.TemporaryDirectory(prefix='dozer-runtime-bench-') as temporary:
    work = Path(temporary)
    src = work / 'bench.rs'
    src.write_text(reference + cursor + main)
    exe = work / 'bench'
    subprocess.run(['rustc', '--edition=2024', '-O', str(src), '-o', str(exe)], check=True)
    print(subprocess.check_output([str(exe)], text=True), end='')
