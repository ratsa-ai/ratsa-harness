//! RATSA-Harness — local, Agent-side entry point to the RATSA.ai platform.
//!
//! One binary, three jobs:
//!
//! 1. **接入** — `install` wires RATSA into whichever Agent you already run
//!    (Claude Code skill / MCP server / Cursor rules / VS Code Copilot /
//!    AGENTS.md / Codex MCP), so the Agent knows how to use the platform.
//! 2. **凭证** — `login` + `keys create --scopes … --bound-slug …` mint a
//!    *least-privilege* key: the permission table decides what it may do, and
//!    the bound handle decides whose account it acts as.
//! 3. **闭环** — `scopes` / `search` / `package` / `pull` / `report` /
//!    `feedback` run the discovery → pull → run-locally → report loop against
//!    the public HTTP API.
//!
//! Design rules: no daemon, no telemetry, no data plane — the CLI only pulls
//! artifacts and pushes structured results. Scanning happens locally.

mod api;
mod commands;
mod config;
mod install;
mod mcp;
mod obs;
mod upgrade;

use clap::{Args, Parser, Subcommand};

use config::Config;

#[derive(Parser, Debug)]
#[command(
    name = "ratsa",
    version,
    about = "ratsa —— RATSA-Harness CLI：把 RATSA.ai 接入你的 Agent（插件/Skill 安装 · 最小权限 key · Harness Package 与 SOF 文件 + 本地评测回传）",
    long_about = None,
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// 安装 / 更新各 Agent 的插件与 Skill（Claude Code、MCP、Cursor、Copilot、AGENTS.md、Codex）
    Install(InstallArgs),
    /// 列出支持的 Agent、各写哪些文件、是否注册 MCP
    Agents {
        /// 输出原始 JSON（供站点/脚本消费）
        #[arg(long)]
        json: bool,
    },
    /// 登录 RATSA 账号（换取会话，用于创建与管理 key）
    Login(LoginArgs),
    /// 退出登录（保留本地 API key）
    Logout,
    /// 查看当前身份、key 权限与归属绑定
    Whoami {
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 打印 RATSA 权限表（可授予 key 的 scope 及含义）
    Scopes,
    /// 管理 API key / secret（权限表 + 归属绑定）
    Keys(KeysArgs),
    /// 本地凭证配置（base_url / key_id / secret）
    Config(ConfigArgs),
    /// 检索 SOF 目录
    Search {
        /// 关键字（名称 / 描述 / slug）
        query: Option<String>,
        /// 按类目过滤，如「核心零部件」
        #[arg(long)]
        category: Option<String>,
        /// 最多返回条数
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 读取一个 SOF 的完整信息
    Sof {
        /// SOF 的 slug（也可粘贴浏览器 URL）
        slug: String,
        /// 读取指定历史版本（semver，如 1.0.0；省略 = 当前版本）
        #[arg(long)]
        version: Option<String>,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 列出一个 SOF 的版本历史
    Versions {
        /// SOF 的 slug
        slug: String,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 读取设备 Harness manifest、就绪度与包清单
    Manifest {
        /// SOF 的 slug
        slug: String,
        /// 按指定历史版本读取（semver；省略 = 当前版本）
        #[arg(long)]
        version: Option<String>,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 只列出可拉取的 Harness Package
    Packages {
        /// SOF 的 slug
        slug: String,
        /// 按指定历史版本读取（semver；省略 = 当前版本）
        #[arg(long)]
        version: Option<String>,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 拉取 Harness Package（设备 Harness 包 / Eval Repo / 服务测试包）
    Pull {
        /// 设备或 Eval Repo / 服务的 slug（也可直接粘贴下载 URL）
        slug: String,
        /// 包类型
        #[arg(long, default_value = "device")]
        kind: String,
        /// 设备 Harness 包按指定历史版本生成（semver；省略 = 当前版本）
        #[arg(long)]
        version: Option<String>,
        /// 落盘目录（默认 ~/.ratsa/packages/<slug>/<kind>）
        #[arg(long)]
        out: Option<String>,
    },
    /// 让平台为该 SOF 生成设备 Harness 包并下载（= pull --kind device）
    Package {
        /// SOF 的 slug
        slug: String,
        /// 按指定历史版本生成（semver；省略 = 当前版本）
        #[arg(long)]
        version: Option<String>,
        /// 只输出描述（包内文件清单 / 就绪度 / 大小），不下载归档
        #[arg(long)]
        meta: bool,
        /// 落盘目录
        #[arg(long)]
        out: Option<String>,
    },
    /// 拉取 SOF 文件（规范化文档，可直接回灌）
    SofFile {
        /// SOF 的 slug
        slug: String,
        /// 拉取指定历史版本（semver；省略 = 当前版本）
        #[arg(long)]
        version: Option<String>,
        /// 落盘目录（默认 ~/.ratsa/sof）
        #[arg(long)]
        out: Option<String>,
    },
    /// 创建或更新 SOF（把 SOF 文件回灌到平台；需 sof:write 权限）
    ///
    /// 输入就是 `sof-file` 拉下来的那份（或手工整理的同类文件），
    /// `component` 块与 `POST/PUT /api/sof` 同构。不传 `--slug` 则新建，
    /// 传了则更新该 slug。
    SofPush {
        /// SOF 文件路径（kind=ratsa.sof.json）
        file: String,
        /// 更新已有 SOF 时传它的 slug；省略则新建
        #[arg(long)]
        slug: Option<String>,
        /// 只校验文件与要提交的内容，不真的提交
        #[arg(long)]
        dry_run: bool,
    },
    /// 上传附件并回填到 SOF 的资源里（直传 OBS，需 sof:write 权限）
    ///
    /// 与 `sof-push` 分工明确：`sof-push` 只推结构化 JSON（资源只有名字没有地址），
    /// 本命令把**本地文件**变成地址并回填 resources。二者不重叠。
    SofUpload {
        /// SOF 的 slug
        slug: String,
        /// 文件或目录；配合 --from-assets 时按 assets/ 分层自动归类
        paths: Vec<String>,
        /// 把路径当作组件目录，按 assets/{docs,drawings,tools}/ 自动定资源类型
        #[arg(long)]
        from_assets: bool,
        /// 资源类型（单选文件时用）；取值同平台的 resource_types
        #[arg(long)]
        r#type: Option<String>,
        /// 只列清单，不请求票据、不上传
        #[arg(long)]
        dry_run: bool,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 回传本地实测结果（Harness Eval，source=agent）
    Report {
        /// 被测设备的 SOF slug
        slug: String,
        /// 实测分数 0-100
        #[arg(long)]
        score: f64,
        /// 通过用例数
        #[arg(long)]
        passed: Option<i64>,
        /// 失败用例数
        #[arg(long)]
        failed: Option<i64>,
        /// 一句话结论
        #[arg(long, default_value = "")]
        summary: String,
        /// 补充说明
        #[arg(long, default_value = "")]
        notes: String,
    },
    /// 提交 Feedback 给厂商
    Feedback {
        /// SOF 的 slug
        slug: String,
        /// 标题
        #[arg(long)]
        title: String,
        /// 详细描述
        #[arg(long, default_value = "")]
        detail: String,
        /// bug | quality | feature | compatibility | support
        #[arg(long, default_value = "")]
        category: String,
        /// low | medium | high | critical
        #[arg(long, default_value = "")]
        severity: String,
    },
    /// 读取 SOF 知识库（厂商用 Markdown 维护；客户凭 k/s 拉取，需 kb:read）
    Kb {
        /// 设备 slug（省略 = 账号级：common + scope）
        #[arg(long)]
        sof: Option<String>,
        /// 只看某一类：common（公共/领域基础）| scope（用户 Scope/账号介绍）| sof（设备）
        #[arg(long)]
        r#type: Option<String>,
        /// 只取某一篇：文档键 key 或文档 slug
        #[arg(long)]
        doc: Option<String>,
        /// 落盘目录（默认把 Markdown 打印到终端）
        #[arg(long)]
        out: Option<String>,
        /// 输出原始 JSON（索引）
        #[arg(long)]
        json: bool,
    },
    /// RATSA Lab（Cloud Lab / 湿实验室）：创建、资料、服务、过往服务与 Spec
    ///
    /// Lab 完全私有：没有公开列表，只能管理本账号（或 key 绑定账号）名下的 Lab。
    /// 任何登录账号都可自助创建并自选领域（bio / chem / env / materials）。
    Lab {
        #[command(subcommand)]
        action: LabAction,
    },
    /// RTH（Ratsa Runtime Harness）：项目、服务通道与 Package 签名验证
    Rth {
        #[command(subcommand)]
        action: RthAction,
    },
    /// 以 MCP stdio 服务器方式运行（供 Agent 作为工具调用）
    Mcp,
    /// 从发布通道更新 `ratsa` 自身
    ///
    /// npm 的 postinstall 只在新装/重装时运行，而包的版本号与 CLI 的 tag 相互独立
    /// （见 npm/README.md 的版本策略）—— 于是只改二进制时，已安装的用户不会收到
    /// 更新。这是那条通道之外唯一的更新入口。
    Upgrade,
}

#[derive(Subcommand, Debug)]
enum RthAction {
    /// 打印 RTH 规范（Ratsa Runtime Harness）
    Spec {
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 列出公开 RTH 项目
    Projects {
        /// 关键字（名称 / 简介）
        #[arg(long)]
        q: Option<String>,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 读取一个 RTH 项目
    Get {
        /// RTH 项目 slug
        slug: String,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 读取服务 → RTH → 设备通道
    Channel {
        /// RTH 项目 slug
        slug: String,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 读取官方 RTH Platform（运行方式 / BSP / Suite / SDK / HAL / OS）
    Platform {
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 列出一个项目的 Release
    Releases {
        /// RTH 项目 slug
        slug: String,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 读取一个 Release（版本 / 通道 / 模式 / 包）
    Release {
        /// RTH 项目 slug
        slug: String,
        /// Release 版本
        version: String,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 创建 Release（owner/admin；管理员可 --publish）
    ReleaseCreate {
        /// RTH 项目 slug
        slug: String,
        /// Release 版本（semver）
        #[arg(long)]
        version: String,
        /// stable | beta | canary
        #[arg(long, default_value = "stable")]
        channel: String,
        /// layer | application | os
        #[arg(long, default_value = "layer")]
        mode: String,
        /// 发布说明
        #[arg(long, default_value = "")]
        notes: String,
        /// 管理员直接发布
        #[arg(long)]
        publish: bool,
    },
    /// 平台管理员发布一个 Release
    Publish {
        /// RTH 项目 slug
        slug: String,
        /// Release 版本
        version: String,
    },
    /// 验证一个 RTH Package 的签名与准入状态
    Verify {
        /// RTH 项目 slug
        slug: String,
        /// Package ID
        package_id: u64,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 在平台登记一个 RTH 项目（需登录）
    Create {
        /// 项目名称
        #[arg(long)]
        name: String,
        /// 一句话介绍
        #[arg(long, default_value = "")]
        summary: String,
        /// 项目说明
        #[arg(long, default_value = "")]
        description: String,
        /// RTHCore 版本
        #[arg(long, default_value = "")]
        core_version: String,
        /// 源码仓库
        #[arg(long, default_value = "")]
        repo: String,
        /// 文档地址
        #[arg(long, default_value = "")]
        docs: String,
        /// 关联 SOF slug
        #[arg(long, default_value = "")]
        sof: String,
        /// 关联 Lab slug
        #[arg(long, default_value = "")]
        lab: String,
        /// public | private
        #[arg(long, default_value = "public")]
        visibility: String,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 登记一个 RTH Package（文件或元数据）
    Package {
        /// RTH 项目 slug
        slug: String,
        /// 包名称
        #[arg(long)]
        name: String,
        /// 包版本
        #[arg(long, default_value = "0.1.0")]
        version: String,
        /// rth-core | rth-plugin | bsp | suite | sdk | hal | os | kernel | layer | perception | ontology | mem | service | other
        #[arg(long, default_value = "rth-plugin")]
        kind: String,
        /// 包文件（可选；省略时需 --checksum）
        #[arg(long)]
        file: Option<String>,
        /// 无文件时的 sha256:<64 hex>
        #[arg(long, default_value = "")]
        checksum: String,
        /// Metadata JSON
        #[arg(long, default_value = "")]
        metadata: String,
        /// 包说明
        #[arg(long, default_value = "")]
        description: String,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 用项目密钥为 Package 加签（需登录）
    Sign {
        /// RTH 项目 slug
        slug: String,
        /// Package ID
        package_id: u64,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 通过 SOF + Lab 验证 Agentic 能力（需登录）
    Validate {
        /// RTH 项目 slug
        slug: String,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 平台管理员准入一个已签名 Package
    Admit {
        /// RTH 项目 slug
        slug: String,
        /// Package ID
        package_id: u64,
        /// 准入备注
        #[arg(long, default_value = "")]
        note: String,
    },
    /// 平台管理员拒绝一个 Package
    Reject {
        /// RTH 项目 slug
        slug: String,
        /// Package ID
        package_id: u64,
        /// 拒绝备注
        #[arg(long, default_value = "")]
        note: String,
    },
    /// 在本地初始化一个 RTH 项目（ratsa.rth.json + .rth/ 状态目录）
    Init {
        /// 项目目录（默认当前目录）
        #[arg(long)]
        dir: Option<String>,
        /// 项目名称（默认目录名）
        #[arg(long)]
        name: Option<String>,
        /// RTHCore 版本
        #[arg(long, default_value = "0.1.0")]
        core_version: String,
    },
    /// 查看本地 RTH 运行时状态
    Status {
        /// 项目目录（默认向上查找 ratsa.rth.json）
        #[arg(long)]
        dir: Option<String>,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 插件安装 / 更新 / 列表（写入 .rth/plugins.json）
    Plugin {
        #[command(subcommand)]
        action: RthPluginAction,
    },
    /// 机器人 Mem（写入 .rth/mem.json）
    Mem {
        #[command(subcommand)]
        action: RthMemAction,
    },
    /// 列出 RTH 清单中的感知部件
    Perception {
        /// 项目目录
        #[arg(long)]
        dir: Option<String>,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 校验 RTH 清单中的 Ontology 定义
    Ontology {
        /// 项目目录
        #[arg(long)]
        dir: Option<String>,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 打印清单中的构建关系（build graph）
    BuildGraph {
        /// 项目目录
        #[arg(long)]
        dir: Option<String>,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
enum RthPluginAction {
    /// 列出本地已安装插件
    List {
        #[arg(long)]
        dir: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// 登记 / 安装一个插件
    Add {
        /// 插件 id
        id: String,
        #[arg(long, default_value = "")]
        version: String,
        /// rth-core | rth-plugin | bsp | suite | sdk | hal | os | kernel | layer | perception | ontology | mem
        #[arg(long, default_value = "rth-plugin")]
        kind: String,
        #[arg(long, default_value = "")]
        source: String,
        #[arg(long, default_value = "stable")]
        channel: String,
        #[arg(long)]
        dir: Option<String>,
    },
    /// 更新插件版本
    Update {
        /// 插件 id
        id: String,
        #[arg(long)]
        version: String,
        #[arg(long)]
        dir: Option<String>,
    },
    /// 移除插件
    Remove {
        /// 插件 id
        id: String,
        #[arg(long)]
        dir: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum RthMemAction {
    /// 列出 Mem 键
    List {
        /// episodic | semantic | working
        #[arg(long, default_value = "working")]
        store: String,
        #[arg(long)]
        dir: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// 读取一个键
    Get {
        key: String,
        #[arg(long, default_value = "working")]
        store: String,
        #[arg(long)]
        dir: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// 写入一个键（值按 JSON 解析，失败则按字符串保存）
    Set {
        key: String,
        value: String,
        #[arg(long, default_value = "working")]
        store: String,
        #[arg(long)]
        dir: Option<String>,
    },
    /// 删除一个键
    Delete {
        key: String,
        #[arg(long, default_value = "working")]
        store: String,
        #[arg(long)]
        dir: Option<String>,
    },
}

#[derive(Args, Debug)]
struct InstallArgs {
    /// 目标 Agent：claude | mcp | cursor | copilot | agents | codex | all
    #[arg(long, short, default_value = "all")]
    agent: String,
    /// 安装到用户目录（默认安装到当前项目）
    #[arg(long, short)]
    global: bool,
    /// 指定安装根目录
    #[arg(long)]
    dir: Option<String>,
    /// 只展示将要写入的内容，不落盘
    #[arg(long)]
    dry_run: bool,
    /// 覆盖已存在的文件
    #[arg(long, short)]
    force: bool,
}

#[derive(Args, Debug)]
struct LoginArgs {
    /// 账号邮箱
    #[arg(long, short)]
    email: String,
    /// 密码（省略则交互式输入）
    #[arg(long, short)]
    password: Option<String>,
    /// 服务端地址（默认取本地配置或 https://ratsa.ai）
    #[arg(long)]
    base_url: Option<String>,
}

#[derive(Args, Debug)]
struct KeysArgs {
    #[command(subcommand)]
    action: KeysAction,
}

#[derive(Subcommand, Debug)]
enum KeysAction {
    /// 列出账号下的 key（含权限与归属绑定）
    List {
        #[arg(long)]
        json: bool,
    },
    /// 创建一个 key：自选权限表条目，可选绑定到某个账号 handle
    Create {
        /// key 名称，便于识别用途（如 laptop-agent、ci）
        #[arg(long, default_value = "agent")]
        name: String,
        /// 权限列表，逗号分隔，可重复：--scopes sof:read,package:pull
        #[arg(long = "scopes", short = 's', value_delimiter = ',')]
        scopes: Vec<String>,
        /// 绑定到某个账号 handle：该 key 之后以该账号身份行动（归属与可操作范围都限定为该账号）
        #[arg(long = "bound-slug", alias = "bind")]
        bound_slug: Option<String>,
        /// 创建后立即设为本地当前凭证
        #[arg(long = "use")]
        use_it: bool,
    },
    /// 吊销一个 key
    Revoke {
        /// key 的数字 id 或 key_id
        id: String,
    },
}

#[derive(Args, Debug)]
struct ConfigArgs {
    #[command(subcommand)]
    action: ConfigAction,
}

#[derive(Subcommand, Debug)]
enum ConfigAction {
    /// 写入 / 更新本地凭证
    Set {
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long)]
        key_id: Option<String>,
        #[arg(long)]
        secret: Option<String>,
        /// 记录归属绑定（仅本地展示用，实际绑定在创建 key 时生效）
        #[arg(long)]
        bound_slug: Option<String>,
    },
    /// 显示当前配置（secret 默认脱敏）
    Show {
        /// 显示完整 secret
        #[arg(long)]
        reveal: bool,
    },
    /// 清除本地 API key（保留会话）
    Clear,
}

#[derive(Subcommand, Debug)]
enum LabAction {
    /// 列出本账号（或 key 绑定账号）名下的 Lab；没有公开列表
    List {
        #[arg(long)]
        json: bool,
    },
    /// 读取一个 Lab（档案 + 服务 + 过往服务 + 资料 + 申请记录）
    Get {
        /// Lab 的 slug
        slug: String,
        #[arg(long)]
        json: bool,
    },
    /// 用 JSON 文件创建 / 更新 Lab 档案；--submit 同时提交审核
    Apply {
        /// 档案 JSON（字段同 POST /api/lab）
        file: String,
        /// 同时提交审核（需先 upload 资料）
        #[arg(long)]
        submit: bool,
        /// 申请备注（配合 --submit）
        #[arg(long, default_value = "")]
        note: String,
        #[arg(long)]
        json: bool,
    },
    /// 查看申请 / 审核状态
    Status {
        /// Lab 的 slug
        slug: String,
        #[arg(long)]
        json: bool,
    },
    /// 删除本账号的 Lab（连同资料、服务与过往服务记录；审核中不可删除）
    Delete {
        /// Lab 的 slug
        slug: String,
    },
    /// 列出 Lab 绑定的 RATSA 设备与运行状态
    Devices {
        /// Lab 的 slug
        slug: String,
        #[arg(long)]
        json: bool,
    },
    /// 从 RATSA AI 目录列出设备厂商与可绑定设备
    Vendors {
        /// 关键字（设备名 / 厂商 / 类目）
        q: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// 绑定一台 RATSA 设备到 Lab（设备来自 RATSA AI 目录）
    DeviceAdd {
        /// Lab 的 slug
        slug: String,
        /// 设备的 SOF slug
        sof: String,
        /// 型号 / 编号
        #[arg(long, default_value = "")]
        model: String,
        /// 备注
        #[arg(long, default_value = "")]
        note: String,
        /// active | idle | maintenance | offline
        #[arg(long, default_value = "active")]
        status: String,
        #[arg(long)]
        json: bool,
    },
    /// 维护设备状态（active | idle | maintenance | offline）
    DeviceStatus {
        /// Lab 的 slug
        slug: String,
        /// 设备 id
        id: u64,
        /// active | idle | maintenance | offline
        status: String,
        /// 顺便更新备注
        #[arg(long)]
        note: Option<String>,
    },
    /// 从 Lab 解绑设备（不删除厂商的 SOF）
    DeviceRemove {
        /// Lab 的 slug
        slug: String,
        /// 设备 id
        id: u64,
    },
    /// 列出客户服务交易反馈（含资质汇总）
    Feedback {
        /// Lab 的 slug
        slug: String,
        #[arg(long)]
        json: bool,
    },
    /// 用实验室账号登记一笔客户交易（平台核验后作为资质 / 营销证据）
    FeedbackAdd {
        /// Lab 的 slug
        slug: String,
        /// 客户联系人
        #[arg(long, default_value = "")]
        customer: String,
        /// 客户公司
        #[arg(long, default_value = "")]
        company: String,
        /// 客户联系方式
        #[arg(long, default_value = "")]
        contact: String,
        /// 关联的 Lab 服务 id（可选）
        #[arg(long)]
        service: Option<u64>,
        /// 订单 / 合同号
        #[arg(long = "order-ref", default_value = "")]
        order_ref: String,
        /// 金额
        #[arg(long, default_value_t = 0.0)]
        amount: f64,
        /// 币种（默认 CNY）
        #[arg(long, default_value = "CNY")]
        currency: String,
        /// 评分 0-5（0 = 暂不评分）
        #[arg(long, default_value_t = 0)]
        rating: u8,
        /// quality | turnaround | support | other
        #[arg(long, default_value = "other")]
        category: String,
        /// 一句话结论
        #[arg(long, default_value = "")]
        title: String,
        /// 详情
        #[arg(long, default_value = "")]
        detail: String,
        /// 交易日期 YYYY-MM-DD（默认为当天）
        #[arg(long, default_value = "")]
        date: String,
        /// 客户已同意作为公开案例 / 营销素材
        #[arg(long)]
        consent: bool,
        #[arg(long)]
        json: bool,
    },
    /// 资质 / 营销汇总（交易数、平均评分、可展示案例）
    FeedbackSummary {
        /// Lab 的 slug
        slug: String,
        #[arg(long)]
        json: bool,
    },
    /// 删除一条交易反馈
    FeedbackDelete {
        /// Lab 的 slug
        slug: String,
        /// 反馈 id
        id: u64,
    },
    /// 上传资质 / 证书 / SOP / 设备清单等资料（单文件 ≤ 25MB）
    Upload {
        /// Lab 的 slug
        slug: String,
        /// 本地文件
        file: String,
        /// license | certificate | sop | equipment | other
        #[arg(long, default_value = "other")]
        kind: String,
        #[arg(long)]
        json: bool,
    },
    /// 拉取 ratsa.lab.json（Agent / 云接入契约，带 sha256 校验）
    Spec {
        /// Lab 的 slug
        slug: String,
        /// 落盘目录（默认 ~/.ratsa/lab）
        #[arg(long)]
        out: Option<String>,
    },
    /// 列出可提供的服务
    Services {
        /// Lab 的 slug
        slug: String,
        #[arg(long)]
        json: bool,
    },
    /// 新增服务（JSON 文件）
    ServiceAdd {
        /// Lab 的 slug
        slug: String,
        /// 服务 JSON（字段同 POST /api/lab/services）
        file: String,
        #[arg(long)]
        json: bool,
    },
    /// 删除服务
    ServiceDelete {
        /// Lab 的 slug
        slug: String,
        /// 服务 id
        id: u64,
    },
    /// 列出过往服务内容
    Records {
        /// Lab 的 slug
        slug: String,
        #[arg(long)]
        json: bool,
    },
    /// 新增过往服务记录（JSON 文件）
    RecordAdd {
        /// Lab 的 slug
        slug: String,
        /// 记录 JSON（字段同 POST /api/lab/records）
        file: String,
        #[arg(long)]
        json: bool,
    },
    /// 删除过往服务记录
    RecordDelete {
        /// Lab 的 slug
        slug: String,
        /// 记录 id
        id: u64,
    },
}

fn main() {
    let cli = Cli::parse();
    let mut cfg = Config::load();
    let result = match cli.command {
        Command::Install(args) => {
            let opt = install::Options {
                agent: args.agent,
                dry_run: args.dry_run,
                global: args.global,
                force: args.force,
                dir: args.dir.map(std::path::PathBuf::from),
            };
            install::run(&cfg, &opt)
        }
        Command::Agents { json } => install::print_agents(json),
        Command::Login(args) => {
            let password = match args.password {
                Some(p) => p,
                None => match rpassword_fallback() {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("错误：{e}");
                        std::process::exit(1);
                    }
                },
            };
            commands::login(&mut cfg, &args.email, &password, args.base_url.as_deref())
        }
        Command::Logout => commands::logout(&mut cfg),
        Command::Whoami { json } => commands::whoami(&mut cfg, json),
        Command::Scopes => commands::scopes(&cfg),
        Command::Keys(args) => match args.action {
            KeysAction::List { json } => commands::keys_list(&cfg, json),
            KeysAction::Create {
                name,
                scopes,
                bound_slug,
                use_it,
            } => commands::keys_create(
                &mut cfg,
                &name,
                &scopes,
                bound_slug.as_deref().unwrap_or(""),
                use_it,
            ),
            KeysAction::Revoke { id } => commands::keys_revoke(&cfg, &id),
        },
        Command::Config(args) => match args.action {
            ConfigAction::Set {
                base_url,
                key_id,
                secret,
                bound_slug,
            } => config_set(&mut cfg, base_url, key_id, secret, bound_slug),
            ConfigAction::Show { reveal } => config_show(&cfg, reveal),
            ConfigAction::Clear => config_clear(&mut cfg),
        },
        Command::Search {
            query,
            category,
            limit,
            json,
        } => run_search(&cfg, query, category, limit, json),
        Command::Sof { slug, version, json } => {
            run_sof(&cfg, &slug, version.as_deref().unwrap_or(""), json)
        }
        Command::Versions { slug, json } => run_versions(&cfg, &slug, json),
        Command::Manifest { slug, version, json } => {
            run_manifest(&cfg, &slug, version.as_deref().unwrap_or(""), json)
        }
        Command::Packages { slug, version, json } => {
            run_packages(&cfg, &slug, version.as_deref().unwrap_or(""), json)
        }
        Command::Pull { slug, kind, version, out } => {
            let client = api::Client::new(&cfg);
            let slug = api::slug_or_url(&slug);
            let out = out.unwrap_or_default();
            commands::pull_report(
                &client,
                &slug,
                &kind,
                version.as_deref().unwrap_or(""),
                &out,
            )
            .map(|s| print!("{s}"))
        }
        Command::Package { slug, version, meta, out } => {
            let client = api::Client::new(&cfg);
            let slug = api::slug_or_url(&slug);
            let out = out.unwrap_or_default();
            commands::package_cmd(
                &client,
                &slug,
                version.as_deref().unwrap_or(""),
                meta,
                &out,
            )
            .map(|s| print!("{s}"))
        }
        Command::SofFile { slug, version, out } => {
            let client = api::Client::new(&cfg);
            let slug = api::slug_or_url(&slug);
            let out = out.unwrap_or_default();
            commands::sof_file_cmd(
                &client,
                &slug,
                version.as_deref().unwrap_or(""),
                &out,
            )
            .map(|s| print!("{s}"))
        }
        Command::SofPush { file, slug, dry_run } => {
            let client = api::Client::new(&cfg);
            commands::sof_push_cmd(&client, &file, slug.as_deref(), dry_run)
                .map(|s| print!("{s}"))
        }
        Command::SofUpload { slug, paths, from_assets, r#type, dry_run, json } => {
            let client = api::Client::new(&cfg);
            let slug = api::slug_or_url(&slug);
            commands::sof_upload_cmd(
                &client, &slug, &paths, from_assets, r#type.as_deref(), dry_run, json,
            )
            .map(|s| print!("{s}"))
        }
        Command::Report {
            slug,
            score,
            passed,
            failed,
            summary,
            notes,
        } => {
            let client = api::Client::new(&cfg);
            let slug = api::slug_or_url(&slug);
            commands::report_run(
                &client, &slug, score, passed, failed, &summary, &notes, "agent",
            )
            .map(|s| print!("{s}"))
        }
        Command::Feedback {
            slug,
            title,
            detail,
            category,
            severity,
        } => {
            let client = api::Client::new(&cfg);
            let slug = api::slug_or_url(&slug);
            commands::submit_feedback(&client, &slug, &title, &detail, &category, &severity)
                .map(|s| print!("{s}"))
        }
        Command::Kb {
            sof,
            r#type,
            doc,
            out,
            json,
        } => {
            let client = api::Client::new(&cfg);
            let sof = sof.unwrap_or_default();
            let tfilter = r#type.unwrap_or_default();
            let out = out.unwrap_or_default();
            match doc {
                Some(d) => commands::kb_read(&client, &sof, &tfilter, &d, &out).map(|s| print!("{s}")),
                None => commands::kb_index(&client, &sof, &tfilter, json).map(|s| print!("{s}")),
            }
        }
        Command::Lab { action } => {
            let client = api::Client::new(&cfg);
            let result = match action {
                LabAction::List { json } => commands::lab_list(&client, json),
                LabAction::Get { slug, json } => commands::lab_get(&client, &slug, json),
                LabAction::Apply {
                    file,
                    submit,
                    note,
                    json,
                } => commands::lab_apply(&client, &file, submit, &note, json),
                LabAction::Status { slug, json } => commands::lab_status(&client, &slug, json),
                LabAction::Delete { slug } => commands::lab_delete(&client, &slug),
                LabAction::Devices { slug, json } => commands::lab_devices(&client, &slug, json),
                LabAction::Vendors { q, json } => {
                    commands::lab_vendors(&client, &q.unwrap_or_default(), json)
                }
                LabAction::DeviceAdd {
                    slug,
                    sof,
                    model,
                    note,
                    status,
                    json,
                } => commands::lab_device_add(&client, &slug, &sof, &model, &note, &status, json),
                LabAction::DeviceStatus {
                    slug,
                    id,
                    status,
                    note,
                } => commands::lab_device_status(&client, &slug, id, &status, note.as_deref()),
                LabAction::DeviceRemove { slug, id } => {
                    commands::lab_device_remove(&client, &slug, id)
                }
                LabAction::Feedback { slug, json } => {
                    commands::lab_feedback(&client, &slug, json)
                }
                LabAction::FeedbackAdd {
                    slug,
                    customer,
                    company,
                    contact,
                    service,
                    order_ref,
                    amount,
                    currency,
                    rating,
                    category,
                    title,
                    detail,
                    date,
                    consent,
                    json,
                } => commands::lab_feedback_add(
                    &client, &slug, &customer, &company, &contact, service, &order_ref,
                    amount, &currency, rating, &category, &title, &detail, &date, consent, json,
                ),
                LabAction::FeedbackSummary { slug, json } => {
                    commands::lab_feedback_summary(&client, &slug, json)
                }
                LabAction::FeedbackDelete { slug, id } => {
                    commands::lab_feedback_delete(&client, &slug, id)
                }
                LabAction::Upload {
                    slug,
                    file,
                    kind,
                    json,
                } => commands::lab_upload(&client, &slug, &file, &kind, json),
                LabAction::Spec { slug, out } => {
                    commands::lab_spec(&client, &slug, &out.unwrap_or_default())
                }
                LabAction::Services { slug, json } => commands::lab_services(&client, &slug, json),
                LabAction::ServiceAdd { slug, file, json } => {
                    commands::lab_service_add(&client, &slug, &file, json)
                }
                LabAction::ServiceDelete { slug, id } => {
                    commands::lab_service_delete(&client, &slug, id)
                }
                LabAction::Records { slug, json } => commands::lab_records(&client, &slug, json),
                LabAction::RecordAdd { slug, file, json } => {
                    commands::lab_record_add(&client, &slug, &file, json)
                }
                LabAction::RecordDelete { slug, id } => {
                    commands::lab_record_delete(&client, &slug, id)
                }
            };
            result.map(|s| print!("{s}"))
        }
        Command::Rth { action } => {
            let client = api::Client::new(&cfg);
            match action {
                RthAction::Spec { json } => commands::rth_spec(&client, json).map(|s| print!("{s}")),
                RthAction::Projects { q, json } => {
                    commands::rth_projects(&client, q.as_deref().unwrap_or(""), json).map(|s| print!("{s}"))
                }
                RthAction::Get { slug, json } => {
                    let slug = api::slug_or_url(&slug);
                    commands::rth_get(&client, &slug, json).map(|s| print!("{s}"))
                }
                RthAction::Channel { slug, json } => {
                    let slug = api::slug_or_url(&slug);
                    commands::rth_channel(&client, &slug, json).map(|s| print!("{s}"))
                }
                RthAction::Platform { json } => commands::rth_platform(&client, json).map(|s| print!("{s}")),
                RthAction::Releases { slug, json } => {
                    let slug = api::slug_or_url(&slug);
                    commands::rth_releases(&client, &slug, json).map(|s| print!("{s}"))
                }
                RthAction::Release { slug, version, json } => {
                    let slug = api::slug_or_url(&slug);
                    commands::rth_release(&client, &slug, &version, json).map(|s| print!("{s}"))
                }
                RthAction::ReleaseCreate { slug, version, channel, mode, notes, publish } => {
                    let slug = api::slug_or_url(&slug);
                    commands::rth_release_create(&client, &slug, &version, &channel, &mode, &notes, publish).map(|s| print!("{s}"))
                }
                RthAction::Publish { slug, version } => {
                    let slug = api::slug_or_url(&slug);
                    commands::rth_publish(&client, &slug, &version).map(|s| print!("{s}"))
                }
                RthAction::Verify { slug, package_id, json } => {
                    let slug = api::slug_or_url(&slug);
                    commands::rth_verify(&client, &slug, package_id, json).map(|s| print!("{s}"))
                }
                RthAction::Create { name, summary, description, core_version, repo, docs, sof, lab, visibility, json } => {
                    commands::rth_create(&client, &name, &summary, &description, &core_version, &repo, &docs, &sof, &lab, &visibility, json).map(|s| print!("{s}"))
                }
                RthAction::Package { slug, name, version, kind, file, checksum, metadata, description, json } => {
                    let slug = api::slug_or_url(&slug);
                    commands::rth_package(&client, &slug, &name, &version, &kind, file.as_deref(), &checksum, &metadata, &description, json).map(|s| print!("{s}"))
                }
                RthAction::Sign { slug, package_id, json } => {
                    let slug = api::slug_or_url(&slug);
                    commands::rth_sign(&client, &slug, package_id, json).map(|s| print!("{s}"))
                }
                RthAction::Validate { slug, json } => {
                    let slug = api::slug_or_url(&slug);
                    commands::rth_validate(&client, &slug, json).map(|s| print!("{s}"))
                }
                RthAction::Admit { slug, package_id, note } => {
                    let slug = api::slug_or_url(&slug);
                    commands::rth_admit(&client, &slug, package_id, &note, false).map(|s| print!("{s}"))
                }
                RthAction::Reject { slug, package_id, note } => {
                    let slug = api::slug_or_url(&slug);
                    commands::rth_admit(&client, &slug, package_id, &note, true).map(|s| print!("{s}"))
                }
                RthAction::Init { dir, name, core_version } => {
                    commands::rth_init(dir.as_deref(), name.as_deref(), &core_version).map(|s| print!("{s}"))
                }
                RthAction::Status { dir, json } => {
                    commands::rth_status(dir.as_deref(), json).map(|s| print!("{s}"))
                }
                RthAction::Plugin { action } => match action {
                    RthPluginAction::List { dir, json } => commands::rth_plugin_list(dir.as_deref(), json).map(|s| print!("{s}")),
                    RthPluginAction::Add { id, version, kind, source, channel, dir } => {
                        commands::rth_plugin_add(dir.as_deref(), &id, &version, &kind, &source, &channel).map(|s| print!("{s}"))
                    }
                    RthPluginAction::Update { id, version, dir } => commands::rth_plugin_update(dir.as_deref(), &id, &version).map(|s| print!("{s}")),
                    RthPluginAction::Remove { id, dir } => commands::rth_plugin_remove(dir.as_deref(), &id).map(|s| print!("{s}")),
                },
                RthAction::Mem { action } => match action {
                    RthMemAction::List { store, dir, json } => commands::rth_mem_list(dir.as_deref(), &store, json).map(|s| print!("{s}")),
                    RthMemAction::Get { key, store, dir, json } => commands::rth_mem_get(dir.as_deref(), &store, &key, json).map(|s| print!("{s}")),
                    RthMemAction::Set { key, value, store, dir } => commands::rth_mem_set(dir.as_deref(), &store, &key, &value).map(|s| print!("{s}")),
                    RthMemAction::Delete { key, store, dir } => commands::rth_mem_delete(dir.as_deref(), &store, &key).map(|s| print!("{s}")),
                },
                RthAction::Perception { dir, json } => commands::rth_perception(dir.as_deref(), json).map(|s| print!("{s}")),
                RthAction::Ontology { dir, json } => commands::rth_ontology(dir.as_deref(), json).map(|s| print!("{s}")),
                RthAction::BuildGraph { dir, json } => commands::rth_build_graph(dir.as_deref(), json).map(|s| print!("{s}")),
            }
        }
        Command::Mcp => mcp::serve(),
        Command::Upgrade => upgrade::run(),
    };

    if let Err(e) = result {
        eprintln!("错误：{e}");
        std::process::exit(1);
    }
}

fn run_search(
    cfg: &Config,
    query: Option<String>,
    category: Option<String>,
    limit: usize,
    json: bool,
) -> Result<(), String> {
    let client = api::Client::new(cfg);
    let q = query.unwrap_or_default();
    let cat = category.unwrap_or_default();
    let rows = commands::filter_sofs(&client, &q, &cat, limit)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::Value::Array(rows)).unwrap_or_default()
        );
        return Ok(());
    }
    print!("{}", commands::format_sofs(&rows));
    Ok(())
}

fn run_sof(cfg: &Config, slug: &str, version: &str, json: bool) -> Result<(), String> {
    let client = api::Client::new(cfg);
    let slug = api::slug_or_url(slug);
    let path = commands::with_version(&format!("/api/sof/{slug}"), version);
    let resp = client.get(&path)?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    if json {
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        return Ok(());
    }
    let item = v.get("item").cloned().unwrap_or(v.clone());
    let name = item.get("name").and_then(|x| x.as_str()).unwrap_or("-");
    let cat = item.get("category").and_then(|x| x.as_str()).unwrap_or("-");
    let desc = item.get("description").and_then(|x| x.as_str()).unwrap_or("");
    let comp_ver = item.get("version").and_then(|x| x.as_str()).unwrap_or("-");
    let std_ver = item.get("sof_version").and_then(|x| x.as_str()).unwrap_or("-");
    println!("{name}  [{cat}]");
    println!("slug      : {slug}");
    println!("版本      : {comp_ver}（SOF 标准 {std_ver}）");
    if !desc.is_empty() {
        println!("描述      : {desc}");
    }
    if let Some(ready) = item.get("agentic_ready").and_then(|x| x.as_bool()) {
        println!("agentic   : {}", if ready { "是" } else { "否（Harness 包为骨架）" });
    }
    if let Some(spec) = item.get("spec").and_then(|x| x.as_str()) {
        if !spec.is_empty() {
            println!("规格      : {}", api::truncate(spec, 400));
        }
    }
    println!();
    println!("下一步：ratsa-harness manifest {slug}   /   ratsa-harness pull {slug}");
    println!("历史  ：ratsa-harness versions {slug}");
    Ok(())
}

fn run_versions(cfg: &Config, slug: &str, json: bool) -> Result<(), String> {
    let client = api::Client::new(cfg);
    let slug = api::slug_or_url(slug);
    let resp = client.get(&format!("/api/sof/{slug}/versions"))?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    if json {
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        return Ok(());
    }
    print!("{}", commands::format_versions(&v));
    Ok(())
}

fn run_manifest(cfg: &Config, slug: &str, version: &str, json: bool) -> Result<(), String> {
    let client = api::Client::new(cfg);
    let slug = api::slug_or_url(slug);
    let path = commands::with_version(&format!("/api/harness/sof/{slug}"), version);
    let resp = client.get(&path)?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    if json {
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        return Ok(());
    }
    if let Some(so) = v.get("sof") {
        let ver = so.get("version").and_then(|x| x.as_str()).unwrap_or("-");
        let std = so.get("sof_version").and_then(|x| x.as_str()).unwrap_or("-");
        println!("设备      : {slug}  ·  版本 {ver}（SOF 标准 {std}）");
    }
    if let Some(rd) = v.get("readiness") {
        let score = rd.get("score").map(|s| s.to_string()).unwrap_or_else(|| "-".into());
        let grade = rd.get("grade").and_then(|x| x.as_str()).unwrap_or("-");
        let valid = rd.get("valid").and_then(|x| x.as_bool()).unwrap_or(false);
        println!("就绪度    : {score} / {grade}{}", if valid { "" } else { "（存在校验问题）" });
        if let Some(issues) = rd.get("issues").and_then(|i| i.as_array()) {
            for it in issues {
                let msg = it
                    .get("message")
                    .or_else(|| it.get("code"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                if !msg.is_empty() {
                    println!("  ! {msg}");
                }
            }
        }
    }
    if let Some(exec) = v.get("execution") {
        let executed = exec.get("executed").and_then(|x| x.as_bool()).unwrap_or(false);
        if executed {
            let score = exec.get("score").map(|s| s.to_string()).unwrap_or_else(|| "-".into());
            let src = exec.get("source").and_then(|x| x.as_str()).unwrap_or("");
            println!("实测      : {score}{}", if src.is_empty() { String::new() } else { format!("（来源 {src}）") });
        } else {
            println!("实测      : 尚未有实测结果（可用 ratsa-harness report 回传）");
        }
    }
    println!();
    print!("{}", commands::format_packages(&v)?);
    Ok(())
}

fn run_packages(cfg: &Config, slug: &str, version: &str, json: bool) -> Result<(), String> {
    let client = api::Client::new(cfg);
    let slug = api::slug_or_url(slug);
    let path = commands::with_version(&format!("/api/harness/sof/{slug}"), version);
    let resp = client.get(&path)?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    if json {
        let pkgs = v.get("packages").cloned().unwrap_or(serde_json::json!([]));
        println!("{}", serde_json::to_string_pretty(&pkgs).unwrap_or_default());
        return Ok(());
    }
    print!("{}", commands::format_packages(&v)?);
    Ok(())
}

fn config_set(
    cfg: &mut Config,
    base_url: Option<String>,
    key_id: Option<String>,
    secret: Option<String>,
    bound_slug: Option<String>,
) -> Result<(), String> {
    if let Some(b) = base_url {
        cfg.base_url = b.trim_end_matches('/').to_string();
    }
    if let Some(k) = key_id {
        cfg.key_id = k.trim().to_string();
    }
    if let Some(s) = secret {
        cfg.secret = s.trim().to_string();
    }
    if let Some(bs) = bound_slug {
        let bs = api::normalize_slug(&bs);
        cfg.bound_slug = bs.clone();
        cfg.acting_as = bs;
    }
    if cfg.key_id.is_empty() != cfg.secret.is_empty() {
        return Err("key_id 与 secret 必须同时提供".into());
    }
    cfg.save()?;
    println!("已保存到 {}", config::config_path().display());
    println!("验证：ratsa-harness whoami");
    Ok(())
}

fn config_show(cfg: &Config, reveal: bool) -> Result<(), String> {
    let path = config::config_path();
    println!("配置文件 : {}", path.display());
    println!("base_url : {}", cfg.base());
    println!("key_id   : {}", if cfg.key_id.is_empty() { "-" } else { &cfg.key_id });
    println!(
        "secret   : {}",
        if cfg.secret.is_empty() {
            "-".to_string()
        } else if reveal {
            cfg.secret.clone()
        } else {
            mask(&cfg.secret)
        }
    );
    println!(
        "权限     : {}",
        if cfg.scopes.is_empty() { "-".to_string() } else { cfg.scopes.join(", ") }
    );
    let bound = if !cfg.acting_as.is_empty() {
        cfg.acting_as.clone()
    } else {
        cfg.bound_slug.clone()
    };
    println!("归属绑定 : {}", if bound.is_empty() { "-".to_string() } else { format!("@{bound}") });
    println!(
        "会话     : {}",
        if cfg.session_token.is_empty() {
            "未登录（无法创建 key：ratsa-harness login --email …）"
        } else {
            "已登录"
        }
    );
    if let Some(u) = cfg.user.as_ref() {
        println!("账号     : {} @{} {}", u.name, u.slug, u.plan);
    }
    Ok(())
}

fn config_clear(cfg: &mut Config) -> Result<(), String> {
    cfg.key_id.clear();
    cfg.secret.clear();
    cfg.scopes.clear();
    cfg.bound_slug.clear();
    cfg.acting_as.clear();
    cfg.key_name.clear();
    cfg.save()?;
    println!("已清除本地 API key 与权限缓存（会话保留）");
    Ok(())
}

fn mask(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 8 {
        return "****".into();
    }
    let head: String = chars.iter().take(6).collect();
    let tail: String = chars.iter().skip(chars.len() - 4).collect();
    format!("{head}…{tail}")
}

/// Password prompt without an extra dependency: read from the TTY with echo on,
/// and prefer `RATSA_PASSWORD` for non-interactive use.
fn rpassword_fallback() -> Result<String, String> {
    if let Ok(p) = std::env::var("RATSA_PASSWORD") {
        if !p.is_empty() {
            return Ok(p);
        }
    }
    eprint!("密码（或设置 RATSA_PASSWORD 环境变量）：");
    use std::io::BufRead;
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    let p = line.trim_end_matches(['\n', '\r']).to_string();
    if p.is_empty() {
        return Err("密码为空".into());
    }
    Ok(p)
}
