#!/bin/bash
# Start AiKv cluster (6 nodes: 3 masters, 3 replicas)

echo "Starting AiKv cluster..."
docker-compose up -d

echo "Waiting for all nodes to be ready..."
sleep 10

# Check if all nodes are up
if docker-compose ps | grep -c "Up" | grep -q "6"; then
    echo "✅ All 6 nodes are running!"
else
    echo "⚠️  Some nodes may not be ready yet. Checking..."
    docker-compose ps
fi

echo ""
echo "To initialize the cluster, run:"
echo "redis-cli --cluster create \\"
echo "  127.0.0.1:6379 127.0.0.1:6380 127.0.0.1:6381 \\"
echo "  127.0.0.1:6382 127.0.0.1:6383 127.0.0.1:6384 \\"
echo "  --cluster-replicas 1"
echo ""
echo "To check cluster status:"
echo "  redis-cli -c -p 6379 CLUSTER INFO"
echo "  redis-cli -c -p 6379 CLUSTER NODES"
