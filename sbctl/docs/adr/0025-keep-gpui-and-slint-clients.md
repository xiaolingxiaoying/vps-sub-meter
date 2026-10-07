# 保留 GPUI 与 Slint + Qt 两套桌面客户端

- **状态：** 已接受。**2026-10-04 部分取代**：Qt Quick/QML 渲染器 `sbgui-qml` 转正为桌面客户端唯一主力与发布物（默认构建、CI 必检、release 产物），GPUI 与 Slint 渲染器随之**冻结**——源码保留但不发布、不修复、不触碰（决策记录见 `docs/legacy-gui-known-issues.md`）。本 ADR 的”保留两套实现”只在”代码保留”层面继续成立；本文其余关于维护与完善两条 renderer 路线的决策不再执行。正式废除留待将来拆除冻结代码的 ADR。
- **日期：** 2026-09-26
- **取代：** ADR-0024 中”Slint + Qt 验收后删除 GPUI 二进制与代码”的迁移收口部分
- **不取代：** ADR-0024 对 Slint + Qt renderer、Windows 原生 IME 与共享控制面边界的决策

## 背景

仓库同时有 GPUI 版本 `sbgui` 和 Slint + Qt 版本 `sbgui-slint`。ADR-0024 为解决 GPUI 中文字形 fallback 与 IME composition 的历史问题，选择 Slint + Qt 作为 Windows renderer，并曾计划验收通过后删除 GPUI。现在的维护要求是同时保留 GPUI 版本和 Slint + Qt 版本，并继续研究和完善 GPUI。

两个实现共用 `client-core`、`ClientSnapshot` / `ClientCommand` seam 和 toolkit-free 的模型/视图逻辑，但各自维护自己的 renderer、控件和平台输入实现。Slint + Qt 的验收不代表 GPUI 的对应行为已通过验收。

## 决策

1. 长期保留 GPUI 和 Slint + Qt 两个桌面实现及其源码路径。
2. 当前二进制名保持 `sbgui`（GPUI）和 `sbgui-slint`（Slint + Qt）；不因 Slint 验收通过而将后者改名覆盖前者。
3. 两种 renderer 继续共享 `client-core` 和 framework-agnostic library；不得把 GPUI 或 Slint 类型引入控制面/共享逻辑。
4. GPUI 的改进先落在应用级平台 adapter、交互行为层和组件层。需要 fork 或更换 GPUI revision 时应先完成单独兼容性评估。
5. 发布产物可以按平台或产品选择其中之一或两者；改变发行默认项不授权删除另一 renderer 的代码。移除任一实现需要另立 ADR 和用户决策。

## 后果

- 可以继续把 GPUI 用作组件、键盘、IME 和框架扩展的研究/产品实现路径，同时保留 Slint + Qt 已验证的 Windows renderer。
- 两套实现增加构建矩阵、截图、依赖和维护成本；各自 renderer 的缺陷与验收记录必须分开。
- 共享逻辑仍有单一数据/命令 seam，防止两个 UI 分叉业务语义。
- `.scratch/sbgui-slint-qt` 中移除 GPUI 的旧工单不再执行；其余 Slint 验收与打包工作不受影响。

## 相关决策与文档

- [ADR-0024：Slint + Qt Windows renderer](0024-sbgui-slint-qt-backend.md)
- [GPUI 当前评估](../research/gpui-current-gui-assessment.md)
- [GPUI 改进路线](../research/gpui-enhancement-strategy.md)
