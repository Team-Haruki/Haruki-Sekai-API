# 云原生改造 TODO

目标：降低 K8s / 容器编排环境下的部署难度（ConfigMap + Secret + 多副本友好），
**完全保留现有本地 YAML 单文件部署方式**——所有改动必须是叠加层，老 yaml 直接启动仍然可用。

## 进度概览

| Phase | 内容 | 状态 |
| --- | --- | --- |
| 1 | 配置加载分层（figment 多源 + `*_file`） | ✅ 完成 |
| 2 | 日志与健康检查（log_format + healthz/readyz） | ✅ 完成 |
| 3 | Updater 拆为独立二进制（`run_updaters_inproc` flag） | ✅ 完成 |
| 4 | 容器与 Helm 资产（非 root Dockerfile + Helm chart + 裸 manifests） | ✅ 完成 |
| 5 | 远程存储抽象（原 Storage trait 设计） | ❌ 已评估，不实施 — 见下文 |
| 5' | 共享卷部署支持（PVC values + 文档） | ✅ 完成（推荐替代方案） |
| 6 | OpenDAL 存储层（显式 opt-in，保留本地默认，含配置文件 bootstrap） | ✅ 完成 |

最新一次回归：`cargo test --lib` 33 passed（1 ignored）、`cargo clippy --all-targets -- -D warnings` clean。

## 兼容性硬约束

- `haruki-sekai-configs.yaml` 现有字段全部保留，含义不变。
- `CONFIG_PATH` 环境变量行为不变；未设置时仍回退到当前目录的 `haruki-sekai-configs.yaml`。
  若设置 `HARUKI_CONFIG_STORAGE__*` / `CONFIG_STORAGE_*`，则启动期通过 OpenDAL 读取配置文件。
- 默认日志、默认端口、默认 cron 表达式、默认 trusted_proxies 等行为不得回归差异。
- 新增字段一律 `#[serde(default)]`，老配置缺省即旧行为。
- 凡引入新源（env / secret 文件），优先级必须是：`默认值 < YAML < 环境变量 < *_FILE`，
  即 YAML 永远可以单独工作；env 仅在显式设置时覆盖。

---

## Phase 1 — 配置加载分层（必做，最高优先级）

可单独发版，落地后即可在 K8s 用 ConfigMap + Secret 部署。

- [x] **P1-1** 在 `Cargo.toml` 增加 `figment = { version = "...", features = ["yaml", "env"] }`
      （或评估保留 `serde_yaml` + 自写 env merger 的方案，二选一并在 PR 描述里说明取舍）。
- [x] **P1-2** 重构 `src/config.rs::Config::load()`：
  - 保持函数签名 `pub fn load() -> anyhow::Result<Self>` 不变。
  - 内部实现改为多源叠加：默认值 → YAML（路径仍由 `CONFIG_PATH` 决定，文件不存在时**降级为可选**而不是报错，但需 `tracing::warn!` 提示）→ 环境变量 `HARUKI_*`（`__` 作为嵌套分隔，例：`HARUKI_BACKEND__PORT`）。
  - 加载完成后调用新函数 `resolve_secret_files(&mut self)`（见 P1-3）。
- [x] **P1-3** 新增 `*_file` 字段族（每个机密字段一一对应，类型 `String`，`#[serde(default)]`）：
  - `BackendConfig::sekai_user_jwt_signing_key_file`
  - `DatabaseConfig::dsn_file`
  - `RedisConfig::password_file`、`RedisConfig::url_file`（见 P1-6）
  - `GitConfig::password_file`、`GitConfig::signing_key_file`
  - `ServerConfig::aes_key_hex_file`、`ServerConfig::aes_iv_hex_file`
  - `AssetUpdaterInfo::authorization_file`
  - `AppHashSource` 如有 token 字段后续也加（当前无）
  实现 `resolve_secret_files`：若 `*_file` 非空，则 `fs::read_to_string` 后 `trim()` 填入对应字段，
  且空文件 / 不存在路径要返回明确错误（`AppError` 或 `anyhow::Error`，见 `src/error.rs`）。
- [x] **P1-4** 编写测试覆盖优先级：
  - 仅 YAML（回归测试，确保 `haruki-sekai-configs.example.yaml` 加载结果不变）。
  - YAML + env 覆盖单字段。
  - YAML + `*_file` 覆盖单字段。
  - env 与 `*_file` 同时存在时 `*_file` 胜出。
- [x] **P1-5** 更新 `haruki-sekai-configs.example.yaml`：**不删字段**，仅在文件顶部加注释说明
      "所有字段均可由 `HARUKI_<UPPER_SNAKE>` env 覆盖；机密字段可用 `*_file` 引用挂载文件。"
- [x] **P1-6**（顺带）`RedisConfig` 增加可选 `url` 字段（`redis://` / `rediss://`）。
      若 `url` 非空则忽略 host/port/password；空则按旧逻辑合成。Loader 内部统一为 URL。
      不要删除老字段。
- [x] **P1-7** README 增加"在 Kubernetes 中部署"小节示例：ConfigMap 装非机密 yaml + Secret 通过 `*_file` 挂载。

---

## Phase 2 — 日志与健康检查（小步，独立）

- [x] **P2-1** `BackendConfig` 增加 `log_format: String`（默认 `"text"`，可选 `"json"`），`#[serde(default)]`。
      在日志初始化处根据该值切换 `tracing_subscriber::fmt::layer()` 的 `.json()`。
- [x] **P2-2** 复核现状：`main_log_file` / `access_log_path` 为空时是否已输出到 stdout。
      若不是，修正为空值默认 stdout（保持非空时写文件的旧行为）。
- [x] **P2-3** 确认 / 新增健康检查路由（在 `src/api/` 下）：
  - `GET /healthz` 总是 200（liveness）。
  - `GET /readyz` 检查 DB（若 `database.enabled`）、Redis（若 `redis.enabled`）连通性，全部 OK 才 200。
  路由必须不经过 JWT 中间件。
- [x] **P2-4** 路由表里把 `/healthz` `/readyz` 加入 access log 排除名单（避免污染日志）。

---

## Phase 3 — Updater 拆分为独立二进制（多副本关键）

目的：API 可水平扩容，updater 单副本独立运行；本地部署可继续 `cargo run` 一把梭。

- [x] **P3-1** 在 `Cargo.toml` 增加 `[[bin]]` 段：
  - `haruki-sekai-api`（默认）：仅启动 HTTP 服务 + 不启动 master / app_hash updater。
  - `haruki-sekai-updater`：仅启动 updater 调度器，不监听 HTTP（或仅暴露 `/healthz`）。
- [x] **P3-2** 在 `BackendConfig` 增加 `run_updaters_inproc: bool`，**默认 `true`**（保留现状：单进程跑全部）。
      仅当用户在 K8s 里把它设成 `false` 才关闭进程内 updater。
      `haruki-sekai-updater` 二进制忽略此字段，永远跑 updater。
- [x] **P3-3** 重构 `src/main.rs` 与现有 `src/updater/` 入口：把 updater 启动逻辑抽到 `pub fn` 给两个二进制共用。
- [x] **P3-4** Dockerfile 同时 COPY 两个二进制；新增 `deploy/k8s/updater-deployment.yaml` 示例（replicas=1）。
- [x] **P3-5** 文档：本地默认行为不变；K8s 推荐 API Deployment（replicas≥2，`run_updaters_inproc=false`）+ Updater Deployment（replicas=1）。

---

## Phase 4 — 容器与 Helm 资产

- [x] **P4-1** Dockerfile 改造：
  - 新增非 root 用户（`USER 65532:65532` 或 distroless nonroot）。
  - `COPY Data ./Data` 保留但允许 `--build-arg INCLUDE_DATA=false` 跳过（默认 true，保持现状）。
  - 验证镜像启动时 `Data/` 不存在的降级路径（如果代码强依赖需另起 issue）。
- [x] **P4-2** 新增 `deploy/helm/haruki-sekai-api/`：
  - `values.yaml` 暴露 image、replicaCount、env、existingSecret、updater.enabled 等。
  - 模板包含 Deployment（API）、Deployment（Updater，可选）、Service、ConfigMap、Secret、可选 Ingress、HPA。
- [x] **P4-3** 新增 `deploy/k8s/` 一组裸 manifests 作为 Helm 之外的最小示例。
- [x] **P4-4** README 顶部加一个"Deployment Matrix"小表：本地 / Docker / Docker Compose / Kubernetes 各自最小步骤。

---

## Phase 5 — 原 Storage trait 方案（已评估，**不实施**）

> 这是早期“从零设计 Storage trait”的评估记录。后续 Phase 6 改为采用 OpenDAL
> 作为薄存储层，且只在显式 `*_storage` 配置时启用。

| 字段 | 实际操作 | 抽象到对象存储是否可行 |
| --- | --- | --- |
| `master_dir` | 写每张 master 表 + 可选 `git push` 整个目录 | OpenDAL 可写 master；`git.enabled=true` 仍必须是真实 local fs 工作树 |
| `account_dir` | 启动加载 + local `notify::PollWatcher` 热重载 | OpenDAL 可 list/read；非 fs 后端改用轮询，fs 后端保留 watcher |
| `version_path` | API 启动/刷新读取，updater 写入 | 适合 OpenDAL，是 API/updater 共享状态的优先目标 |
| `nuverse_structure_file_path` | Nuverse master updater 读取 | 适合 OpenDAL |

**结论更新**：不自建 trait；使用 OpenDAL 封装 read/write/list/stat。PVC 仍是最简单的
K8s 方案，但对象存储现在是可选部署路径。

- [ ] ~~**P5-1** 设计 `Storage` trait~~（不实施）
- [ ] ~~**P5-2** `LocalFs` 实现~~（不实施）
- [ ] ~~**P5-3** ServerConfig 路径字段升级为 URI~~（不实施）
- [ ] ~~**P5-4** `object_store` 接 S3 / GCS~~（不实施）
- [x] **P5-5** Git push 路径保持 LocalFs only — 现状即是，文档已在 README / Helm values 注释中体现

---

## Phase 5' — 共享卷部署支持（替代方案，**完成**）

实际可行的多副本路径：用 K8s PVC 把 `master_dir` / `account_dir` 挂成跨 pod 共享卷。

- [x] **P5'-1** Helm chart 暴露 `persistence.master.{enabled, existingClaim, mountPath, accessMode, size, storageClass}`
      与 `persistence.accounts.{...}`，分别挂到 updater / API Deployment。
- [x] **P5'-2** `deploy/k8s/all-in-one.yaml` 加 PVC 挂载示例（默认注释，按需启用）。
- [x] **P5'-3** README "Configuration sources" 后增 "Persistent storage" 小节，说明：
  - `master_dir`：updater 写、API 不读 → updater 单副本即可，**RWO 足够**；只有当外部还有读者（如 git remote 镜像）才需 RWX。
  - `account_dir`：多 API 副本共享 → 必须 RWX（NFS / Longhorn / EFS / Filestore）。
  - `version_path`：API 读、updater 写；可用共享卷或 `version_storage`。
  - `nuverse_structure_file_path`：updater 读；可与 `master_dir` 同卷或使用 `nuverse_structure_storage`。
- [x] **P5'-4** Helm `values.yaml` 默认 `persistence.*.enabled: false` 保留无状态启动可能（适合开发环境 / 单副本）。

---

## Phase 6 — OpenDAL 存储层（重新评估后实施）

边界：旧路径字段继续默认本地文件语义；只有显式配置 `*_storage` 时才走 OpenDAL。

- [x] **P6-1** 引入 OpenDAL，并提升 MSRV 到 Rust 1.85。
- [x] **P6-2** 增加 `StorageConfig` 与 secret-file 支持。
- [x] **P6-3** `version_path` / `nuverse_structure_file_path` 走 storage wrapper。
- [x] **P6-4** `master_dir` 写入和 DB ingest 走 storage wrapper；`git.enabled=true` 仍要求 local fs。
- [x] **P6-5** `account_dir` 支持 local fs watcher；非 fs storage 使用 OpenDAL polling。
- [x] **P6-6** `apphash_sources[type=file]` 支持 OpenDAL storage。
- [x] **P6-7** `CONFIG_PATH` 支持通过启动期 `config_storage` / `CONFIG_STORAGE_*` 指向 OpenDAL `fs`。
- [x] **P6-8** 跑完整回归并补充必要测试。

---

## 不在本计划内（已评估，暂不做）

- 进程内 leader election（Redis SETNX 或 K8s Lease）：被 Phase 3 的"updater 拆二进制"替代，更简单。
  若未来需要 updater 也水平扩容再考虑。
- TLS 在进程内取消：保留 `backend.ssl` 字段，文档建议生产由 Ingress 承担即可。
- 把 `Data/` 内容外置到 OCI artifact / sidecar：先观察 P4-1 build-arg 是否足够。

---

## 验收清单（每个 Phase 合并前自检）

- [x] `haruki-sekai-configs.example.yaml` 原样加载（`config::tests::loads_example_yaml_unchanged` 锁定基线）。
- [x] `cargo test --lib` 全绿（33 passed，1 ignored），新增字段均有缺省回退测试。
- [x] `cargo clippy --all-targets -- -D warnings` 无新增告警。
- [x] README "How to Use" 段落（本地部署）保持原状未改动。
