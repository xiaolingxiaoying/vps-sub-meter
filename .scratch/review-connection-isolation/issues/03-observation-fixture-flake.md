# 观测命令的 Unix 测试夹具出现一次偶发失败
Status: needs-triage
Type: research

v0.0.16 本地完整门禁第一次运行时，`observe_sing_box_process_reads_a_rooted_systemctl_fixture` 返回的 main_pid 为 None，预期 Some(7)。当时断言未输出 observation.error，尚不能确认是进程启动、夹具命令还是解析失败。

随后该测试单独复测通过，完整 487 项测试也通过；同提交 Release Actions 的 ci/test 通过。没有因该现象修改生产观测逻辑，也没有把一次重试当作根因修复。后续需先加强断言诊断，再确认可重复触发条件。

## Comments

2026-10-20 下一轮：测试现在先断言并输出 observation.error，再检查 PID 并携带完整观测对象。使用 32 测试线程连续运行完整库测试五轮，全部通过，未稳定复现。此次 host command 边界修复单独有可重复证据，不能据此声称本偶发问题已解决；保持 needs-triage，没有增加自动重试掩盖失败。
