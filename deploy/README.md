# AiKv Cluster Deployment

This directory contains deployment files for a 6-node AiKv cluster (3 masters, 3 replicas).

## Prerequisites

- Docker and Docker Compose installed
- AiKv cluster Docker image built:
  ```bash
  docker build -t aikv:cluster --build-arg FEATURES=cluster .
  ```

## Files

| File | Description |
|------|-------------|
| docker-compose.yml | Cluster Docker Compose configuration |
| aikv-node[1-6].toml | Per-node configuration files |
| start.sh | Start cluster script |
| stop.sh | Stop cluster script |
| init-cluster.sh | Initialize cluster script |

## Quick Start

```bash
# 1. Start all nodes
./start.sh

# 2. Wait for nodes to be ready, then initialize cluster
./init-cluster.sh
```

## Manual Steps

```bash
# Start cluster
docker-compose up -d

# Initialize cluster (after all nodes are up)
redis-cli --cluster create \
  127.0.0.1:6379 127.0.0.1:6380 127.0.0.1:6381 \
  127.0.0.1:6382 127.0.0.1:6383 127.0.0.1:6384 \
  --cluster-replicas 1

# Check cluster status
redis-cli -c -p 6379 CLUSTER INFO
redis-cli -c -p 6379 CLUSTER NODES
```

## Connecting

```bash
# Connect with cluster mode
redis-cli -c -p 6379

# Test with hash tags (ensures keys go to same slot)
redis-cli -c -p 6379 SET {user:1000}:name "John"
redis-cli -c -p 6379 SET {user:1000}:age "30"
```

## Node Ports

| Node | Data Port | Cluster Port | Role |
|------|-----------|--------------|------|
| aikv1 | 6379 | 16379 | Master |
| aikv2 | 6380 | 16380 | Master |
| aikv3 | 6381 | 16381 | Master |
| aikv4 | 6382 | 16382 | Replica |
| aikv5 | 6383 | 16383 | Replica |
| aikv6 | 6384 | 16384 | Replica |

## Cluster Operations

```bash
# Check cluster info
redis-cli -c -p 6379 CLUSTER INFO

# View nodes
redis-cli -c -p 6379 CLUSTER NODES

# View slot distribution
redis-cli -c -p 6379 CLUSTER SLOTS

# Get key slot
redis-cli -c -p 6379 CLUSTER KEYSLOT mykey

# Manual failover (on replica node)
redis-cli -p 6382 CLUSTER FAILOVER
```

## Monitoring

```bash
# View all logs
docker-compose logs -f

# View specific node logs
docker-compose logs -f aikv1

# Check node status
docker-compose ps
```

## Stopping

```bash
./stop.sh

# Or manually
docker-compose down

# Remove all data
docker-compose down -v
```
