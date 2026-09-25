#!/bin/sh
# Sample the memory of the PostgreSQL backends serving one application_name,
# for measuring an ingest against a throwaway (docker) PostgreSQL:
#
#   tools/pg_backend_memory_sampler.sh <container> <application_name> [log_above_kb]
#
# Give the ingest target's DSN `?application_name=<name>`. Every 0.2 s this
# prints "epoch pid RssAnon_kB <process title>" for each backend of that
# application; backends above `log_above_kb` (default 150000) that are not
# idle get `pg_log_backend_memory_contexts(pid)`, whose context dump goes to
# the server log (`docker logs <container> | grep -A300 'logging memory
# contexts of PID <pid>'`). The superuser is taken from $PGUSER (postgres).
# Stop it with Ctrl-C.
set -u
C=$1
APP=$2
LIM=${3:-150000}
U=${PGUSER:-postgres}
while :; do
  pids=$(docker exec "$C" psql -U "$U" -d postgres -tAc \
    "SELECT string_agg(pid::text, ' ') FROM pg_stat_activity WHERE application_name = '$APP'")
  if [ -n "$pids" ]; then
    now=$(docker exec "$C" sh -c "for p in $pids; do \
      echo \"\$(date +%s) \$p \$(awk '/RssAnon/{print \$2}' /proc/\$p/status 2>/dev/null) \
\$(tr '\\0' ' ' < /proc/\$p/cmdline 2>/dev/null)\"; done")
    echo "$now"
    echo "$now" | awk -v l="$LIM" '$3 > l && $0 !~ / idle *$/ {print $2}' | while read -r p; do
      docker exec "$C" psql -U "$U" -d postgres -tAc \
        "SELECT pg_log_backend_memory_contexts($p)" >/dev/null
    done
  fi
  sleep 0.2
done
