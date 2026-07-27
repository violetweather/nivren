#!/bin/sh
set -eu

versions="6.2 7.2 7.4 8.0 8.2 8.4 8.6 8.8"

for version in $versions; do
    name="nivren-redis-matrix-$(printf '%s' "$version" | tr '.' '-')-$$"
    docker run --detach --name "$name" --publish 127.0.0.1::6379 "redis:${version}-alpine" \
        redis-server --save '' --appendonly no >/dev/null
    cleanup() {
        docker rm --force "$name" >/dev/null 2>&1 || true
    }
    trap cleanup EXIT INT TERM

    attempts=0
    until docker exec "$name" redis-cli ping 2>/dev/null | grep -q PONG; do
        attempts=$((attempts + 1))
        if [ "$attempts" -ge 100 ]; then
            echo "Redis $version did not become ready" >&2
            exit 1
        fi
        sleep 0.1
    done

    port=$(docker port "$name" 6379/tcp | head -n 1 | awk -F: '{ print $NF }')
    echo "Testing Nivren Redis client against Redis $version on port $port"
    NIVREN_REDIS_PORT="$port" cargo test --test language \
        official_redis_live_release_matrix -- --ignored --exact

    cleanup
    trap - EXIT INT TERM
done
