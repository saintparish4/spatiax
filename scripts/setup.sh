#!/bin/bash

# High-Speed Telemetry System Setup Script
# This script sets up the system for optimal performance

set -e

echo "🚀 Setting up High-Speed Telemetry System..."

# Check if running as root
if [[ $EUID -eq 0 ]]; then
   echo "This script should not be run as root for security reasons."
   echo "Please run as a regular user with sudo privileges."
   exit 1
fi

# Function to check if a command exists
command_exists() {
    command -v "$1" >/dev/null 2>&1
}

# Install system dependencies
install_dependencies() {
    echo "📦 Installing system dependencies..."
    
    if command_exists apt-get; then
        # Ubuntu/Debian
        sudo apt-get update
        sudo apt-get install -y \
            build-essential \
            pkg-config \
            libssl-dev \
            can-utils \
            iproute2 \
            curl \
            git \
            htop \
            iotop \
            sysstat
    elif command_exists yum; then
        # CentOS/RHEL
        sudo yum update -y
        sudo yum install -y \
            gcc \
            gcc-c++ \
            make \
            pkg-config \
            openssl-devel \
            can-utils \
            iproute \
            curl \
            git \
            htop \
            iotop \
            sysstat
    else
        echo "❌ Unsupported package manager. Please install dependencies manually."
        exit 1
    fi
}

# Install Rust
install_rust() {
    if ! command_exists rustc; then
        echo "🦀 Installing Rust..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        source ~/.cargo/env
        rustup update
    else
        echo "✅ Rust is already installed"
        rustc --version
    fi
}

# Install Docker
install_docker() {
    if ! command_exists docker; then
        echo "🐳 Installing Docker..."
        curl -fsSL https://get.docker.com -o get-docker.sh
        sudo sh get-docker.sh
        sudo usermod -aG docker $USER
        rm get-docker.sh
        
        # Install Docker Compose
        sudo curl -L "https://github.com/docker/compose/releases/latest/download/docker-compose-$(uname -s)-$(uname -m)" -o /usr/local/bin/docker-compose
        sudo chmod +x /usr/local/bin/docker-compose
    else
        echo "✅ Docker is already installed"
        docker --version
    fi
}

# Configure system for high-performance telemetry
configure_system() {
    echo "⚙️ Configuring system for high performance..."
    
    # Increase file descriptor limits
    echo "* soft nofile 1048576" | sudo tee -a /etc/security/limits.conf
    echo "* hard nofile 1048576" | sudo tee -a /etc/security/limits.conf
    
    # Increase network buffer sizes
    echo "net.core.rmem_max = 134217728" | sudo tee -a /etc/sysctl.conf
    echo "net.core.wmem_max = 134217728" | sudo tee -a /etc/sysctl.conf
    echo "net.core.netdev_max_backlog = 5000" | sudo tee -a /etc/sysctl.conf
    
    # Optimize for low latency
    echo "net.core.busy_poll = 50" | sudo tee -a /etc/sysctl.conf
    echo "net.core.busy_read = 50" | sudo tee -a /etc/sysctl.conf
    
    # Increase shared memory limits
    echo "kernel.shmmax = 17179869184" | sudo tee -a /etc/sysctl.conf
    echo "kernel.shmall = 4194304" | sudo tee -a /etc/sysctl.conf
    
    # Apply sysctl changes
    sudo sysctl -p
}

# Setup CAN interfaces
setup_can() {
    echo "🚗 Setting up CAN interfaces..."
    
    # Load CAN kernel modules
    sudo modprobe can
    sudo modprobe can_raw
    sudo modprobe vcan
    
    # Make modules load at boot
    echo "can" | sudo tee -a /etc/modules
    echo "can_raw" | sudo tee -a /etc/modules
    echo "vcan" | sudo tee -a /etc/modules
    
    # Create virtual CAN interface for testing
    sudo ip link add dev vcan0 type vcan
    sudo ip link set up vcan0
    
    # Create systemd service for CAN setup
    sudo tee /etc/systemd/system/telemetry-can.service > /dev/null << 'EOF'
[Unit]
Description=Telemetry CAN Interface Setup
After=network.target

[Service]
Type=oneshot
ExecStart=/bin/bash -c 'modprobe can && modprobe can_raw && modprobe vcan && ip link add dev vcan0 type vcan && ip link set up vcan0'
RemainAfterExit=yes

[Install]
WantedBy=multi-user.target
EOF
    
    sudo systemctl enable telemetry-can.service
    sudo systemctl start telemetry-can.service
}

# Create directory structure
create_directories() {
    echo "📁 Creating directory structure..."
    
    mkdir -p config data logs models
    mkdir -p config/grafana config/prometheus
    
    # Set appropriate permissions
    chmod 755 config data logs models
}

# Generate configuration files
generate_configs() {
    echo "📝 Generating configuration files..."
    
    # Create sample prometheus config
    cat > config/prometheus.yml << 'EOF'
global:
  scrape_interval: 15s

scrape_configs:
  - job_name: 'telemetry-system'
    static_configs:
      - targets: ['telemetry-system:8080']
    metrics_path: '/metrics'
    scrape_interval: 5s

  - job_name: 'node-exporter'
    static_configs:
      - targets: ['localhost:9100']
EOF

    # Create MQTT config
    mkdir -p config/mosquitto
    cat > config/mosquitto/mosquitto.conf << 'EOF'
listener 1883
allow_anonymous true
persistence true
persistence_location /mosquitto/data/
log_dest file /mosquitto/log/mosquitto.log
log_type error
log_type warning
log_type notice
log_type information
EOF
}

# Build the system
build_system() {
    echo "🔨 Building telemetry system..."
    
    if [ -f "Cargo.toml" ]; then
        cargo build --release
        echo "✅ Build completed successfully"
    else
        echo "❌ Cargo.toml not found. Make sure you're in the project root directory."
        exit 1
    fi
}

# Setup systemd service
setup_service() {
    echo "🔧 Setting up systemd service..."
    
    # Get current directory
    INSTALL_DIR=$(pwd)
    
    sudo tee /etc/systemd/system/telemetry-system.service > /dev/null << EOF
[Unit]
Description=High-Speed Telemetry Processing System
After=network.target

[Service]
Type=simple
User=$USER
Group=$USER
WorkingDirectory=$INSTALL_DIR
ExecStart=$INSTALL_DIR/target/release/telemetry-system
Restart=always
RestartSec=5
Environment=RUST_LOG=info
Environment=TELEMETRY_CONFIG=$INSTALL_DIR/config/telemetry.json

# Resource limits
LimitNOFILE=1048576
LimitNPROC=1048576

# Security settings
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=strict
ReadWritePaths=$INSTALL_DIR/data $INSTALL_DIR/logs

[Install]
WantedBy=multi-user.target
EOF
    
    sudo systemctl daemon-reload
    sudo systemctl enable telemetry-system.service
}

# Performance tuning
performance_tuning() {
    echo "🚀 Applying performance tuning..."
    
    # CPU governor
    if [ -f /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor ]; then
        echo "performance" | sudo tee /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor
    fi
    
    # Disable swap for better performance
    sudo swapoff -a
    
    # Optimize scheduler
    echo "1" | sudo tee /sys/kernel/debug/sched_features
}

# Main installation process
main() {
    echo "🎯 High-Speed Telemetry System Setup"
    echo "===================================="
    
    install_dependencies
    install_rust
    install_docker
    configure_system
    setup_can
    create_directories
    generate_configs
    build_system
    setup_service
    performance_tuning
    
    echo ""
    echo "✅ Setup completed successfully!"
    echo ""
    echo "📋 Next steps:"
    echo "  1. Review and customize config/telemetry.json"
    echo "  2. Start the system: sudo systemctl start telemetry-system"
    echo "  3. Check status: sudo systemctl status telemetry-system"
    echo "  4. View logs: journalctl -u telemetry-system -f"
    echo "  5. Access dashboard: http://localhost:8080"
    echo ""
    echo "🔧 For Docker deployment:"
    echo "  docker-compose up -d"
    echo ""
    echo "⚠️  Please reboot your system to apply all kernel parameter changes."
}

# Run main function
main "$@"
