#!/usr/bin/env python3
"""Run named TCP snapshot mutations; restore source even when a test fails."""
import json, os, pathlib, subprocess, sys
root=pathlib.Path(__file__).resolve().parents[3]
source=root/'esp-soc/src/nat.rs'
original=source.read_text()
start=original.index('    pub fn tcp_flow_counts(')
end=original.index('    /// Forward a UDP',start)
method=original[start:end]
mutations=[
 ('omit-connecting', 'Transport::Connecting(_) | Transport::Connected(_) => counts.active += 1,', 'Transport::Connecting(_) => {},\n                Transport::Connected(_) => counts.active += 1,', 'connecting_snapshot_does_not_consume_ready_or_refused_results'),
 ('omit-connected', 'Transport::Connecting(_) | Transport::Connected(_) => counts.active += 1,', 'Transport::Connecting(_) => counts.active += 1,\n                Transport::Connected(_) => {},', 'guest_reset_is_excluded_before_poll_without_pruning'),
 ('closed-as-active', 'Transport::Closed => {}', 'Transport::Closed => counts.active += 1,', 'guest_reset_is_excluded_before_poll_without_pruning'),
 ('time-wait-as-active', 'Transport::TimeWait => counts.time_wait += 1,', 'Transport::TimeWait => counts.active += 1,', 'half_close_unacked_fin_and_final_ack_retry_remain_occupied'),
 ('omit-time-wait', 'Transport::TimeWait => counts.time_wait += 1,', 'Transport::TimeWait => {},', 'half_close_unacked_fin_and_final_ack_retry_remain_occupied'),
 ('drop-half-close', 'Transport::Connecting(_) | Transport::Connected(_) => counts.active += 1,', 'Transport::Connecting(_) | Transport::Connected(_) => { if flow.guest_write == GuestWrite::Open { counts.active += 1; } },', 'half_close_unacked_fin_and_final_ack_retry_remain_occupied'),
 ('drop-unacked-fin', 'Transport::Connecting(_) | Transport::Connected(_) => counts.active += 1,', 'Transport::Connecting(_) | Transport::Connected(_) => { if !flow.host_closed { counts.active += 1; } },', 'half_close_unacked_fin_and_final_ack_retry_remain_occupied'),
 ('consume-connector-result', 'Transport::Connecting(_) | Transport::Connected(_) => counts.active += 1,', 'Transport::Connecting(ref rx) => { let _ = rx.try_recv(); counts.active += 1; },\n                Transport::Connected(_) => counts.active += 1,', 'connecting_snapshot_does_not_consume_ready_or_refused_results'),
 ('cumulative-not-occupancy', 'let mut counts = TcpFlowCounts::default();', 'let mut counts = TcpFlowCounts { active: self.tcp_opened as usize, time_wait: 0 };', 'mixed_snapshot_counts_states_not_cumulative_statistics'),
 ('mutable-snapshot-contract', 'pub fn tcp_flow_counts(&self)', 'pub fn tcp_flow_counts(&mut self)', 'empty_and_failed_admission_have_no_tcp_occupancy'),
]
output=pathlib.Path(sys.argv[1]) if len(sys.argv)>1 else root/'target/tcp-counts-mutations'
output.mkdir(parents=True,exist_ok=True)
rows=[]
try:
 for name,old,new,test in mutations:
    assert method.count(old)==1, (name,old)
    source.write_text(original[:start]+method.replace(old,new)+original[end:])
    cmd=['nice','-n','10','cargo','+1.99.0','test','--release','-j2','-p','esp-soc','nat::tests::counts::'+test,'--','--exact']
    with (output/(name+'.log')).open('w') as log:
        p=subprocess.run(cmd,cwd=root,stdout=log,stderr=subprocess.STDOUT)
    text=(output/(name+'.log')).read_text()
    contract=name=='mutable-snapshot-contract'
    killed=p.returncode!=0 and (('error[E0596]' in text) if contract else ('test result: FAILED' in text and test in text))
    rows.append(dict(name=name,test=test,exit_code=p.returncode,killed=killed,kind='compile contract' if contract else 'runtime assertion'))
    print(name,killed,flush=True)
    (output/'results.json').write_text(json.dumps(rows,indent=2)+'\n')
    if not killed: raise SystemExit('surviving mutation: '+name)
finally:
 source.write_text(original)
p=subprocess.run(['nice','-n','10','cargo','+1.99.0','test','--release','-j2','-p','esp-soc','nat::tests'],cwd=root)
raise SystemExit(p.returncode)
