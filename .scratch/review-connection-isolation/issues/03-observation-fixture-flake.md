# 观测命令的 Unix 测试夹具出现一次偶发失败
Status: needs-triage
Type: research

v0.0.16 本地完整门禁第一次运行时，`observe_sing_box_process_reads_a_rooted_systemctl_fixture` 返回的 main_pid 为 None，预期 Some(7)。当时断言未输出 observation.error，尚不能确认是进程启动、夹具命令还是解析失败。

随后该测试单独复测通过，完整 487 项测试也通过；同提交 Release Actions 的 ci/test 通过。没有因该现象修改生产观测逻辑，也没有把一次重试当作根因修复。后续需先加强断言诊断，再确认可重复触发条件。
