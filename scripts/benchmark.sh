#!/bin/bash
# Performance benchmarking script for AiKv
# This script runs redis-benchmark against AiKv server and generates performance reports

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Configuration
HOST=${AIKV_HOST:-127.0.0.1}
PORT=${AIKV_PORT:-6379}
REQUESTS=${AIKV_REQUESTS:-100000}
CLIENTS=${AIKV_CLIENTS:-50}
PIPELINE=${AIKV_PIPELINE:-1}
KEY_SIZE=${AIKV_KEY_SIZE:-3}
VALUE_SIZE=${AIKV_VALUE_SIZE:-256}
OUTPUT_DIR=${AIKV_OUTPUT_DIR:-./benchmark_results}

echo -e "${GREEN}=== AiKv Performance Benchmark ===${NC}"
echo "Configuration:"
echo "  Host: $HOST"
echo "  Port: $PORT"
echo "  Requests: $REQUESTS"
echo "  Clients: $CLIENTS"
echo "  Pipeline: $PIPELINE"
echo "  Key Size: $KEY_SIZE bytes"
echo "  Value Size: $VALUE_SIZE bytes"
echo ""

# Create output directory
mkdir -p "$OUTPUT_DIR"
TIMESTAMP=$(date +"%Y%m%d_%H%M%S")
REPORT_FILE="$OUTPUT_DIR/benchmark_$TIMESTAMP.txt"

# Check if redis-benchmark is available
if ! command -v redis-benchmark &> /dev/null; then
    echo -e "${YELLOW}Warning: redis-benchmark not found. Installing redis-tools...${NC}"
    sudo apt-get update && sudo apt-get install -y redis-tools || {
        echo -e "${RED}Failed to install redis-tools. Please install it manually.${NC}"
        exit 1
    }
fi

# Check if server is running
echo -e "${YELLOW}Checking if AiKv server is running at $HOST:$PORT...${NC}"
if ! nc -z "$HOST" "$PORT" 2>/dev/null; then
    echo -e "${RED}Server is not running at $HOST:$PORT${NC}"
    echo "Please start the server first with: cargo run --release"
    exit 1
fi

echo -e "${GREEN}Server is running. Starting benchmark...${NC}"
echo ""

# Run redis-benchmark
{
    echo "=== AiKv Performance Benchmark Report ==="
    echo "Timestamp: $(date)"
    echo "Configuration: requests=$REQUESTS, clients=$CLIENTS, pipeline=$PIPELINE"
    echo ""
    echo "=== Command Performance ==="
    echo ""
} > "$REPORT_FILE"

# Benchmark SET command
echo -e "${YELLOW}Benchmarking SET...${NC}"
redis-benchmark -h "$HOST" -p "$PORT" -t set -n "$REQUESTS" -c "$CLIENTS" -P "$PIPELINE" \
    -d "$VALUE_SIZE" --csv 2>&1 | tee -a "$REPORT_FILE"

# Benchmark GET command
echo -e "${YELLOW}Benchmarking GET...${NC}"
redis-benchmark -h "$HOST" -p "$PORT" -t get -n "$REQUESTS" -c "$CLIENTS" -P "$PIPELINE" \
    -d "$VALUE_SIZE" --csv 2>&1 | tee -a "$REPORT_FILE"

# Benchmark INCR command
echo -e "${YELLOW}Benchmarking INCR...${NC}"
redis-benchmark -h "$HOST" -p "$PORT" -t incr -n "$REQUESTS" -c "$CLIENTS" -P "$PIPELINE" \
    --csv 2>&1 | tee -a "$REPORT_FILE"

# Benchmark LPUSH command (if supported)
echo -e "${YELLOW}Benchmarking LPUSH...${NC}"
redis-benchmark -h "$HOST" -p "$PORT" -t lpush -n "$REQUESTS" -c "$CLIENTS" -P "$PIPELINE" \
    --csv 2>&1 | tee -a "$REPORT_FILE" || echo "LPUSH not supported" | tee -a "$REPORT_FILE"

# Benchmark LPOP command (if supported)
echo -e "${YELLOW}Benchmarking LPOP...${NC}"
redis-benchmark -h "$HOST" -p "$PORT" -t lpop -n "$REQUESTS" -c "$CLIENTS" -P "$PIPELINE" \
    --csv 2>&1 | tee -a "$REPORT_FILE" || echo "LPOP not supported" | tee -a "$REPORT_FILE"

# Benchmark SADD command (if supported)
echo -e "${YELLOW}Benchmarking SADD...${NC}"
redis-benchmark -h "$HOST" -p "$PORT" -t sadd -n "$REQUESTS" -c "$CLIENTS" -P "$PIPELINE" \
    --csv 2>&1 | tee -a "$REPORT_FILE" || echo "SADD not supported" | tee -a "$REPORT_FILE"

# Benchmark HSET command (if supported)
echo -e "${YELLOW}Benchmarking HSET...${NC}"
redis-benchmark -h "$HOST" -p "$PORT" -t hset -n "$REQUESTS" -c "$CLIENTS" -P "$PIPELINE" \
    --csv 2>&1 | tee -a "$REPORT_FILE" || echo "HSET not supported" | tee -a "$REPORT_FILE"

# Benchmark PING command
echo -e "${YELLOW}Benchmarking PING...${NC}"
redis-benchmark -h "$HOST" -p "$PORT" -t ping -n "$REQUESTS" -c "$CLIENTS" -P "$PIPELINE" \
    --csv 2>&1 | tee -a "$REPORT_FILE"

# Benchmark MSET command
echo -e "${YELLOW}Benchmarking MSET...${NC}"
redis-benchmark -h "$HOST" -p "$PORT" -t mset -n "$REQUESTS" -c "$CLIENTS" -P "$PIPELINE" \
    -d "$VALUE_SIZE" --csv 2>&1 | tee -a "$REPORT_FILE"

echo ""
echo -e "${GREEN}Benchmark complete!${NC}"
echo -e "Results saved to: ${GREEN}$REPORT_FILE${NC}"

# Generate summary
echo ""
echo "=== Performance Summary ===" | tee -a "$REPORT_FILE"
grep -E "\"(SET|GET|INCR|PING|MSET)\"" "$REPORT_FILE" | tail -10 | tee -a "$REPORT_FILE"

echo ""
echo -e "${GREEN}Done!${NC}"
