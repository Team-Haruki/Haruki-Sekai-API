> [!Caution]
> This project was rewritten in Rust.  
> Go edition and Python edition are not maintained anymore.  
> If you want to use Python edition, please go to [old python branch](https://github.com/Team-Haruki/Haruki-Sekai-API/tree/old-python).  
> If you want to use Go edition, please go to [old go branch](https://github.com/Team-Haruki/Haruki-Sekai-API/tree/old-go).

# Haruki Sekai API

**Haruki Sekai API** is a companion project for [HarukiBot](https://github.com/Team-Haruki), providing direct API access to various servers of the game `Project Sekai: Colorful Stage`.

## Requirements
+ `MySQL`, `SQLite`, `PostgreSQL` (Optional, depending on your database choice)
+ `Redis` (Optional, for caching sekai users)

## How to Use
1. Go to release page to download `haruki-sekai-api`
2. Rename `haruki-sekai-configs.example.yaml` to `haruki-sekai-configs.yaml` and then edit it.
3. Make a new directory or use an exists directory
4. Put `haruki-sekai-api` and `haruki-sekai-configs.yaml` in the same directory
5. Edit `haruki-sekai-configs.yaml` and configure it
6. Open Terminal, and `cd` to the directory
7. Run `haruki-sekai-api`

## Deployment matrix

| Target              | Recipe                                                                                                                        |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| Local / single host | Edit `haruki-sekai-configs.yaml` and run `./haruki-sekai-api`. Updater runs in-process (`backend.run_updaters_inproc=true`).  |
| Single Docker       | `docker run -v ./haruki-sekai-configs.yaml:/app/haruki-sekai-configs.yaml -p 9999:9999 ghcr.io/team-haruki/haruki-sekai-api`. |
| Kubernetes (Helm)   | `helm install haruki deploy/helm/haruki-sekai-api -f my-values.yaml` — see `deploy/helm/haruki-sekai-api/values.yaml`.        |
| Kubernetes (raw)    | `kubectl apply -f deploy/k8s/all-in-one.yaml` — minimal copy-edit example (see `deploy/k8s/README.md`).                       |

The Kubernetes targets run two Deployments off the same image: an
horizontally-scaled API (`./haruki-sekai-api`) with
`backend.run_updaters_inproc=false`, plus a singleton updater
(`./haruki-sekai-updater`, `replicas: 1`) that owns scheduled jobs.

## Configuration sources

`haruki-sekai-api` loads configuration from the following sources, in order of
increasing priority. Local single-file deployments only need step 2 — the rest
are opt-in for container / Kubernetes use.

1. Built-in defaults.
2. The YAML file pointed to by `CONFIG_PATH` (default
   `./haruki-sekai-configs.yaml`).
3. Environment variables prefixed with `HARUKI_`. Nested keys use `__` as the
   separator. Examples:
   - `HARUKI_BACKEND__PORT=9999`
   - `HARUKI_REDIS__URL=redis://:pass@redis.svc:6379/0`
   - `HARUKI_SERVERS__JP__AES_KEY_HEX=...`
4. `*_file` fields. For any sensitive setting `foo`, you may set `foo_file`
   to a path; the trimmed contents of that file replace `foo`. This is the
   intended way to consume Kubernetes Secret / Docker secret mounts.
   Available: `database.dsn_file`, `master_database.dsn_file`,
   `redis.password_file`, `redis.url_file`,
   `backend.sekai_user_jwt_signing_key_file`,
   `git.password_file`, `git.signing_key_file`,
   `servers.<region>.aes_key_hex_file`,
   `servers.<region>.aes_iv_hex_file`,
   `servers.<region>.*_storage.secret_access_key_file`,
   `servers.<region>.*_storage.access_key_secret_file`,
   `apphash_sources[N].storage.secret_access_key_file`,
   `apphash_sources[N].storage.access_key_secret_file`,
   `asset_updater_servers[N].authorization_file`.

If `CONFIG_PATH` is unset and the default YAML file is missing, the service
starts from defaults + env vars only (a warning is logged). If `CONFIG_PATH`
is explicitly set but points at a missing file, startup fails fast.

### Persistent storage

Kubernetes deployments can stay stateless by default, but multi-replica
installations usually need PVCs for paths that are backed by files:

| Path setting | Recommended volume | Notes |
| ------------ | ------------------ | ----- |
| `servers.<region>.account_dir` | RWX PVC mounted on API and updater | Required when API replicas share account files; the updater also initializes clients and reads this directory. Use NFS, Longhorn, EFS, Filestore, or another RWX-capable StorageClass. |
| `servers.<region>.master_dir` | RWO PVC mounted on updater | The API does not read this path. The singleton updater writes master data and may run `git push`, so a normal filesystem is required. |
| `servers.<region>.version_path` | Shared API/updater volume or `version_storage` | API clients read it on startup and version refresh; the updater writes it. |
| `servers.<region>.nuverse_structure_file_path` | Same updater PVC as `master_dir` or `nuverse_structure_storage` | Used by the Nuverse master updater. |

The Helm chart exposes `persistence.accounts.*` and `persistence.master.*`;
both are disabled by default. After enabling a PVC, set the matching server
path fields through YAML or `HARUKI_SERVERS__<REGION>__...` env vars so they
point under the configured mount path.

### OpenDAL storage

Selected path-backed settings can use OpenDAL instead of local files by adding
the storage block next to the legacy path. If the storage block is omitted, the
old path behavior is unchanged.

| Legacy path | Optional storage block | Notes |
| ----------- | ---------------------- | ----- |
| `account_dir` | `account_storage` | `scheme: fs` keeps the local `notify` watcher. Other schemes use periodic OpenDAL `list/stat` polling, controlled by `poll_interval_secs`. |
| `master_dir` | `master_storage` | Updater writes master JSON files through OpenDAL. `git.enabled=true` still requires local fs storage because git needs a real working tree. |
| `version_path` | `version_storage` | Read by API clients and written by updater, so this is the best shared-state target for object storage. |
| `nuverse_structure_file_path` | `nuverse_structure_storage` | Read by the Nuverse master updater. |
| `apphash_sources[N].dir` | `apphash_sources[N].storage` | Used when `type: file`; reads `{region}.json` from the configured storage prefix. |

Supported schemes in the default binary are `fs`, `s3`, `oss`, `cos`, `gcs`,
`azblob`, and `obs`.

Example:

```yaml
servers:
  jp:
    account_storage:
      scheme: "s3"
      bucket: "haruki-sekai"
      root: "accounts/jp"
      endpoint: "https://s3.example.com"
      region: "auto"
      access_key_id: "..."
      secret_access_key_file: "/run/secrets/haruki/s3_secret"
      poll_interval_secs: 30
    version_storage:
      scheme: "s3"
      bucket: "haruki-sekai"
      root: "versions"
      path: "jp/version.json"
      endpoint: "https://s3.example.com"
      region: "auto"
      access_key_id: "..."
      secret_access_key_file: "/run/secrets/haruki/s3_secret"
```

### Kubernetes deployment sketch

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: haruki-sekai-config
data:
  HARUKI_BACKEND__PORT: "9999"
  HARUKI_REDIS__ENABLED: "true"
  HARUKI_REDIS__URL: "redis://redis.default.svc:6379/0"
---
apiVersion: v1
kind: Secret
metadata:
  name: haruki-sekai-secrets
type: Opaque
stringData:
  jwt_key: "replace-me"
  db_dsn: "postgres://user:pass@pg/db?sslmode=disable"
  jp_aes_key: "deadbeef..."
  jp_aes_iv:  "cafebabe..."
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: haruki-sekai-api
spec:
  replicas: 1
  selector: { matchLabels: { app: haruki-sekai-api } }
  template:
    metadata: { labels: { app: haruki-sekai-api } }
    spec:
      containers:
        - name: api
          image: ghcr.io/team-haruki/haruki-sekai-api:latest
          envFrom:
            - configMapRef: { name: haruki-sekai-config }
          env:
            - name: HARUKI_BACKEND__SEKAI_USER_JWT_SIGNING_KEY_FILE
              value: /run/secrets/haruki/jwt_key
            - name: HARUKI_DATABASE__DSN_FILE
              value: /run/secrets/haruki/db_dsn
            - name: HARUKI_SERVERS__JP__AES_KEY_HEX_FILE
              value: /run/secrets/haruki/jp_aes_key
            - name: HARUKI_SERVERS__JP__AES_IV_HEX_FILE
              value: /run/secrets/haruki/jp_aes_iv
          volumeMounts:
            - { name: secrets, mountPath: /run/secrets/haruki, readOnly: true }
          ports: [{ containerPort: 9999 }]
      volumes:
        - name: secrets
          secret: { secretName: haruki-sekai-secrets }
```

No `haruki-sekai-configs.yaml` is mounted — the deployment runs entirely from
ConfigMap + Secret. To layer a partial YAML on top, mount it at any path and
set `CONFIG_PATH` to point at it.

## License

This project is licensed under the MIT License.
