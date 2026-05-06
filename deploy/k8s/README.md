# Kubernetes manifests

`all-in-one.yaml` is a minimal, copy-edit-and-apply example that demonstrates
the recommended cloud-native topology:

- **API Deployment** (`replicas: 2`) — horizontally scalable, cron-free.
- **Updater Deployment** (`replicas: 1`, `strategy: Recreate`) — singleton that
  owns master-data download, app-hash polling, and git push.

Both Deployments run from the same image; the only difference is the `command`
(`./haruki-sekai-api` vs `./haruki-sekai-updater`).

For a parameterized install, use the Helm chart at `deploy/helm/haruki-sekai-api`
instead.

## What you must edit before applying

1. Replace placeholders in the `Secret` (search for `REPLACE_ME` / `REPLACE_HEX`).
   In production, manage that Secret out-of-band (sealed-secrets, external-secrets,
   SOPS, etc.) and remove the inline `stringData` block.
2. Pin the `image:` tag — `latest` is shown for brevity only.
3. Add `HARUKI_SERVERS__<REGION>__*` env vars for every region you plan to enable.
4. If your cluster uses a different non-root UID convention, adjust
   `runAsUser` / `runAsGroup` / `fsGroup` accordingly (defaults to 65532).
5. If you run multiple API replicas, uncomment the `haruki-sekai-accounts` PVC
   example and use an RWX StorageClass (NFS, Longhorn, EFS, Filestore, etc.).
   The `haruki-sekai-master` PVC is only used by the singleton updater, so RWO
   is sufficient unless another workload also needs concurrent access.
