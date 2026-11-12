# Failover: egress relays and standby nodes

Two independent layers keep a region's game API calls working when the node that holds
its accounts is in trouble.

| Failure | Layer | What happens |
|---|---|---|
| The game refuses the account node's IP, or its route to the game is down | Egress failover | The account node sends game traffic through another node's IP. Accounts and sessions stay where they are. |
| The account node itself is down | Standby node | A standby that already holds the accounts serves the calls. Callers need its address as a backup. |

A relogin loop is what led to the first kind of failure on 2026-11-12 (about five logins
a second after a data update), so the client also refuses to loop: see "Login gate" below.

## Roles

| Role | Binary | Holds accounts | Config |
|---|---|---|---|
| Master | `haruki-sekai-api` | Yes, the account files | `servers.<region>.egress`, `account_sync.peers` |
| Egress relay | `egress_relay` | No | `egress_relay` |
| Standby | `haruki-sekai-api` | A pushed copy, logged out | `account_sync.receive`, `upstreams` |

One node can run a relay and a standby side by side; they are separate processes.

## Layer 1: egress failover

On the master, list the ways out for a region, primary first:

```yaml
servers:
  cn:
    egress:
      - name: local
        url: ""                      # direct from this node
      - name: relay-a
        url: "http://relay:<token>@<relay-a private ip>:9996"
      - name: relay-b
        url: "http://relay:<token>@<relay-b private ip>:9996"
    egress_failback: auto            # or manual
```

On each relay node (a config file with just this and `backend: {}`):

```yaml
egress_relay:
  listen: "<private ip>:9996"
  token: "<token>"
  allow_hosts:
    - "<the region's game API host>"
```

How the master decides:

1. Logins fail on the active route for two accounts in a row (or game calls get no HTTP
   response at all).
2. The same account logs in once through up to two standby routes.
3. A standby accepts the login: the region moves to it. Every other session logs in again
   through the new route before its next call.
4. No standby accepts it: the route is not the problem (a stale app identity looks the
   same from every IP). The region stays, closes the login gate, and leaves the standby
   routes alone for 30 minutes.

With `egress_failback: auto` the master logs one account in through the primary every
30 minutes (doubling to 6 hours while it keeps failing) and returns when that works.

Only game API traffic follows the active route. CDN downloads and public thumbnails keep
using the node's default client.

### Login gate

- A relogin that succeeds but is followed by the same rejection is not repeated: one
  relogin per request and error kind.
- After repeated rejections the gate closes. Logins then fail fast, with one probe let
  through per backoff (30 s, doubling to 5 min). A login that never reached the game
  (a network error) does not count: there is nothing to shed, and the region must
  recover the moment the network does.
- During a data update, calls that were sent with the old data version retry without
  logging in once another call has brought the new version in. Expect about one relogin
  per account, not one per request.
- A pushed app identity (`POST /internal/app-identity`) or an account reload resets the
  gate, so recovery is immediate once the cause is fixed.

### Operating it

```bash
# State of every local region: active route, gate, last error, switches, sessions
curl -H "Authorization: Bearer $INTERNAL_TOKEN" http://<master>:9999/internal/egress

# Move a region and keep it there (no automatic failback until hold is cleared)
curl -X PUT -H "Authorization: Bearer $INTERNAL_TOKEN" -H 'Content-Type: application/json' \
  -d '{"server":"cn","egress":"relay-a","hold":true}' http://<master>:9999/internal/egress

# Release the hold; failback resumes on its schedule
curl -X PUT -H "Authorization: Bearer $INTERNAL_TOKEN" -H 'Content-Type: application/json' \
  -d '{"server":"cn","hold":false}' http://<master>:9999/internal/egress
```

`GET /health` (no auth) lists each local region with `sessions`, `egress`, `primary`,
`gated` and `lastSuccessAt`, and reports `status: degraded` when a region has no sessions
or its gate is closed. Alert on `degraded`, and on `primary: false` if a region running on
a standby route should page someone.

## Layer 2: standby node

On the master:

```yaml
servers:
  cn:
    account_sync:
      peers:
        - name: standby-a
          url: "http://<standby private ip>:9999"
          token: "<standby's backend.internal_token>"
```

On the standby:

```yaml
backend:
  internal_token: "<token the master pushes with>"
servers:
  cn:
    enabled: true
    account_dir: "./Data/accounts/cn"     # owned by the push; starts empty
    account_sync:
      receive: true
    local_priority: 10
    upstreams:
      - name: master
        url: "http://<master private ip>:9999"
        token: "<master's backend.internal_token>"
        priority: 0
```

- The master pushes its whole account set at startup, after every account-file change,
  and every 10 minutes. Nothing can pull accounts from the master.
- **The push carries the account credentials and the standby's token in the request.**
  Like every node-to-node call in this service it is plain HTTP by default, so the peer
  URL must be an address on an encrypted overlay (a VPN such as WireGuard), or an
  `https://` URL. A private IP on an unencrypted network is not enough. Redirects are
  refused, so a peer cannot bounce the account set elsewhere.
- Removing every account file on the master clears the standby's copy too. A missing
  account directory or files with no usable account do not: that looks like a broken
  mount, and the standby keeps what it has.
- The standby stores the set in `synced-accounts.json` (owner-only), loads nothing else
  from that directory, and does not log in.
- A call that reaches the standby goes to the master first. Only when the master cannot
  serve it does the standby log an account in and serve the call itself. When the master
  is back, calls go to it again and its sessions log in again on their next use.
- The standby needs the region's AES keys, version file and (Nuverse) schema bundle like
  any node that serves the region. App identity pushes must reach it too: add it to the
  registry's `account_nodes`.

Callers reach the standby only if they know its address. Each caller needs the standby's
URL as a backup that is used when the master's address does not answer.
