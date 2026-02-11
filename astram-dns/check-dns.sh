#!/bin/bash
# DNS server script to check the status of the Astram DNS server

# DNS_SERVER=http://dns.Astram.com:8053 ./check-dns.sh
DNS_SERVER=${DNS_SERVER:-"http://161.33.19.183:8053"}

echo "=== Astram DNS Server Status ==="
echo ""

# Health check
echo "1. Health Check:"
curl -s $DNS_SERVER/health | jq . 2>/dev/null || curl -s $DNS_SERVER/health
echo ""
echo ""

# Stats
echo "2. Statistics:"
curl -s $DNS_SERVER/stats | jq . 2>/dev/null || curl -s $DNS_SERVER/stats
echo ""
echo ""

# Nodes
echo "3. Registered Nodes:"
curl -s $DNS_SERVER/nodes | jq . 2>/dev/null || curl -s $DNS_SERVER/nodes
echo ""

