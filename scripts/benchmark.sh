#!/bin/bash

# High-Speed Telemetry System Benchmark Script
# Tests system performance under various load conditions

set -e

echo "🏁 Telemetry System Performance Benchmark"
echo "========================================"

# Configuration
BENCHMARK_DURATION=60  # seconds
MAX_CHANNELS=1000
SAMPLE_RATES=(10 100 1000 5000)  # Hz
BATCH_SIZES=(100 500 1000 5000)

# Results directory
RESULTS_DIR="benchmark_results_$(date +%Y%m%d_%H%M%S)"
mkdir -p "$RESULTS_DIR"

# Check if system is running
check_system() {
    if ! pgrep -f "telemetry-system" > /dev/null; then
        echo "❌ Telemetry system is not running. Please start it first."
        echo "   sudo systemctl start telemetry-system"
        echo "   or"
        echo "   cargo run --release"
        exit 1
    fi
    echo "✅ Telemetry system is running"
}

# Generate test data
generate_can_data() {
    local sample_rate=$1
    local duration=$2
    local output_file=$3
    
    echo "📊 Generating CAN test data at ${sample_rate} Hz for ${duration}s..."
    
    # Use cansend to generate test data
    (
        for ((i=0; i<$((sample_rate * duration)); i++)); do
            # Engine RPM (ID: 0x100)
            rpm=$((1000 + RANDOM % 5000))
            printf "%08X#%04X000000000000\n" 0x100 $rpm
            
            # Vehicle Speed (ID: 0x200)  
            speed=$((RANDOM % 300))
            printf "%08X#%04X000000000000\n" 0x200 $speed
            
            # Engine Temp (ID: 0x300)
            temp=$((60 + RANDOM % 50))
            printf "%08X#%02X00000000000000\n" 0x300 $temp
            
            # Sleep to maintain sample rate
            sleep $(echo "scale=6; 1.0/$sample_rate" | bc -l) 2>/dev/null || sleep 0.001
        done
    ) > "$output_file"
}

# Generate UDP test data
generate_udp_data() {
    local sample_rate=$1
    local duration=$2
    local channels=$3
    
    echo "📊 Generating UDP test data: ${channels} channels at ${sample_rate} Hz for ${duration}s..."
    
    python3 << EOF
import socket
import time
import json
import random
import threading

def send_data():
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    
    start_time = time.time()
    samples_sent = 0
    
    while time.time() - start_time < $duration:
        # Create batch of samples
        samples = []
        for channel in range(1, $channels + 1):
            sample = {
                "channel": channel,
                "value": random.uniform(-100, 100),
                "timestamp": int(time.time() * 1e9),  # nanoseconds
                "quality": "good"
            }
            samples.append(sample)
        
        # Send batch
        message = {
            "samples": samples,
            "timestamp": int(time.time() * 1e9)
        }
        
        data = json.dumps(message).encode('utf-8')
        sock.sendto(data, ('localhost', 8888))
        
        samples_sent += len(samples)
        
        # Control sample rate
        time.sleep(1.0 / $sample_rate)
    
    sock.close()
    print(f"Sent {samples_sent} samples")

send_data()
EOF
}

# Monitor system performance
monitor_performance() {
    local duration=$1
    local output_file=$2
    
    echo "📈 Monitoring system performance for ${duration}s..."
    
    # Start monitoring in background
    (
        echo "timestamp,cpu_percent,memory_mb,ingestion_rate,processing_latency" > "$output_file"
        
        for ((i=0; i<duration; i++)); do
            # Get system metrics
            cpu=$(top -bn1 | grep "Cpu(s)" | awk '{print $2}' | sed 's/%us,//')
            memory=$(ps -o pid,vsz,comm -p $(pgrep -f telemetry-system) | tail -1 | awk '{print $2/1024}')
            
            # Get application metrics via HTTP API (if available)
            ingestion_rate=$(curl -s http://localhost:8080/metrics 2>/dev/null | grep "ingestion_rate" | awk '{print $2}' || echo "0")
            latency=$(curl -s http://localhost:8080/metrics 2>/dev/null | grep "processing_latency" | awk '{print $2}' || echo "0")
            
            timestamp=$(date +%s)
            echo "$timestamp,$cpu,$memory,$ingestion_rate,$latency" >> "$output_file"
            
            sleep 1
        done
    ) &
    
    local monitor_pid=$!
    return $monitor_pid
}

# Run CAN bus benchmark
benchmark_can() {
    echo ""
    echo "🚗 CAN Bus Benchmark"
    echo "==================="
    
    for sample_rate in "${SAMPLE_RATES[@]}"; do
        echo ""
        echo "Testing CAN at ${sample_rate} Hz..."
        
        # Generate test data
        test_file="$RESULTS_DIR/can_test_${sample_rate}hz.candump"
        generate_can_data $sample_rate $BENCHMARK_DURATION "$test_file"
        
        # Start performance monitoring
        perf_file="$RESULTS_DIR/can_perf_${sample_rate}hz.csv"
        monitor_performance $BENCHMARK_DURATION "$perf_file" &
        monitor_pid=$!
        
        # Send CAN data
        echo "📡 Sending CAN data..."
        canplayer -I "$test_file" vcan0=can0 &
        player_pid=$!
        
        # Wait for test completion
        sleep $BENCHMARK_DURATION
        
        # Stop processes
        kill $player_pid 2>/dev/null || true
        kill $monitor_pid 2>/dev/null || true
        wait 2>/dev/null || true
        
        echo "✅ CAN test at ${sample_rate} Hz completed"
    done
}

# Run UDP benchmark
benchmark_udp() {
    echo ""
    echo "🌐 UDP Network Benchmark" 
    echo "======================="
    
    local test_channels=(10 50 100 500 1000)
    
    for channels in "${test_channels[@]}"; do
        for sample_rate in "${SAMPLE_RATES[@]}"; do
            echo ""
            echo "Testing UDP: ${channels} channels at ${sample_rate} Hz..."
            
            # Start performance monitoring
            perf_file="$RESULTS_DIR/udp_perf_${channels}ch_${sample_rate}hz.csv"
            monitor_performance $BENCHMARK_DURATION "$perf_file" &
            monitor_pid=$!
            
            # Generate and send UDP data
            generate_udp_data $sample_rate $BENCHMARK_DURATION $channels &
            generator_pid=$!
            
            # Wait for test completion
            wait $generator_pid
            
            # Stop monitoring
            kill $monitor_pid 2>/dev/null || true
            wait 2>/dev/null || true
            
            echo "✅ UDP test: ${channels} channels at ${sample_rate} Hz completed"
            
            # Brief pause between tests
            sleep 2
        done
    done
}

# Run stress test
stress_test() {
    echo ""
    echo "💪 Stress Test"
    echo "=============="
    
    local stress_duration=300  # 5 minutes
    local max_sample_rate=10000
    local max_channels=1000
    
    echo "Running maximum load test: ${max_channels} channels at ${max_sample_rate} Hz for ${stress_duration}s..."
    
    # Start performance monitoring
    perf_file="$RESULTS_DIR/stress_test.csv"
    monitor_performance $stress_duration "$perf_file" &
    monitor_pid=$!
    
    # Generate maximum load
    generate_udp_data $max_sample_rate $stress_duration $max_channels &
    generator_pid=$!
    
    # Wait for completion
    wait $generator_pid
    kill $monitor_pid 2>/dev/null || true
    wait 2>/dev/null || true
    
    echo "✅ Stress test completed"
}

# Analyze results
analyze_results() {
    echo ""
    echo "📊 Analyzing Results"
    echo "==================="
    
    # Generate summary report
    report_file="$RESULTS_DIR/benchmark_report.txt"
    
    cat > "$report_file" << EOF
High-Speed Telemetry System Benchmark Report
============================================

Test Date: $(date)
System Info: $(uname -a)
CPU Info: $(grep "model name" /proc/cpuinfo | head -1 | cut -d: -f2 | xargs)
Memory: $(free -h | grep "Mem:" | awk '{print $2}')

Test Configuration:
- Benchmark Duration: ${BENCHMARK_DURATION}s
- Sample Rates Tested: ${SAMPLE_RATES[*]} Hz
- Max Channels: ${MAX_CHANNELS}

Results Summary:
===============

EOF

    # Analyze performance data
    echo "Performance Analysis:" >> "$report_file"
    
    for perf_file in "$RESULTS_DIR"/*.csv; do
        if [[ -f "$perf_file" ]]; then
            filename=$(basename "$perf_file" .csv)
            echo "" >> "$report_file"
            echo "Test: $filename" >> "$report_file"
            echo "-------------------" >> "$report_file"
            
            # Calculate statistics
            if command -v python3 >/dev/null 2>&1; then
                python3 << EOF >> "$report_file"
import csv
import statistics

try:
    with open('$perf_file', 'r') as f:
        reader = csv.DictReader(f)
        data = list(reader)
    
    if data:
        cpu_values = [float(row['cpu_percent']) for row in data if row['cpu_percent'].replace('.','').isdigit()]
        memory_values = [float(row['memory_mb']) for row in data if row['memory_mb'].replace('.','').isdigit()]
        
        if cpu_values:
            print(f"Average CPU: {statistics.mean(cpu_values):.1f}%")
            print(f"Max CPU: {max(cpu_values):.1f}%")
        
        if memory_values:
            print(f"Average Memory: {statistics.mean(memory_values):.1f} MB")
            print(f"Max Memory: {max(memory_values):.1f} MB")
    else:
        print("No data available")
        
except Exception as e:
    print(f"Analysis error: {e}")
EOF
            fi
        fi
    done
    
    echo ""
    echo "📄 Benchmark report saved to: $report_file"
    echo "📁 All results available in: $RESULTS_DIR/"
}

# Generate plots (if gnuplot is available)
generate_plots() {
    if command -v gnuplot >/dev/null 2>&1; then
        echo ""
        echo "📈 Generating performance plots..."
        
        # Create gnuplot script for CPU usage
        cat > "$RESULTS_DIR/plot_cpu.gp" << 'EOF'
set terminal png size 1200,800
set output 'cpu_usage.png'
set title 'CPU Usage Over Time'
set xlabel 'Time (seconds)'
set ylabel 'CPU Usage (%)'
set grid
set datafile separator ','
plot 'stress_test.csv' using 1:2 with lines title 'CPU Usage'
EOF
        
        cd "$RESULTS_DIR"
        gnuplot plot_cpu.gp 2>/dev/null || echo "Could not generate CPU plot"
        cd - >/dev/null
        
        echo "📊 Plots saved to $RESULTS_DIR/"
    fi
}

# Main benchmark execution
main() {
    echo "Starting comprehensive benchmark suite..."
    echo "Results will be saved to: $RESULTS_DIR"
    echo ""
    
    # Check prerequisites
    check_system
    
    # Install Python3 if needed for UDP tests
    if ! command -v python3 >/dev/null 2>&1; then
        echo "❌ Python3 is required for UDP benchmarks"
        exit 1
    fi
    
    # Run benchmarks
    benchmark_can
    benchmark_udp
    stress_test
    
    # Analyze and report
    analyze_results
    generate_plots
    
    echo ""
    echo "🎉 Benchmark completed successfully!"
    echo "📊 Results available in: $RESULTS_DIR/"
    echo ""
    echo "Key files:"
    echo "  - benchmark_report.txt: Summary report"
    echo "  - *.csv: Raw performance data"
    echo "  - *.png: Performance plots (if available)"
}

# Run main function
main "$@"
