#!/bin/bash
# Initialize AiKv cluster

echo "Initializing AiKv cluster..."

redis-cli --cluster create \
  127.0.0.1:6379 127.0.0.1:6380 127.0.0.1:6381 \
  127.0.0.1:6382 127.0.0.1:6383 127.0.0.1:6384 \
  --cluster-replicas 1

echo ""
echo "Cluster initialization complete!"
echo ""
echo "Checking cluster status..."
redis-cli -c -p 6379 CLUSTER INFO
