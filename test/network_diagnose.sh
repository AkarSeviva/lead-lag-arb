#!/bin/bash
# 网络诊断脚本

echo "=========================================="
echo "网络诊断"
echo "=========================================="

echo ""
echo "[1] DNS 解析测试"
echo "----------------------------------------"
nslookup uuapi.rerrkvifj.com

echo ""
echo "[2] 检查环境变量中的代理"
echo "----------------------------------------"
echo "HTTP_PROXY: $HTTP_PROXY"
echo "HTTPS_PROXY: $HTTPS_PROXY"
echo "http_proxy: $http_proxy"
echo "https_proxy: $https_proxy"
echo "NO_PROXY: $NO_PROXY"
echo "no_proxy: $no_proxy"

echo ""
echo "[3] 直连测试 (curl 无代理)"
echo "----------------------------------------"
curl -v --noproxy '*' -X POST 'https://uuapi.rerrkvifj.com/cfd/agg/v1/sendQryAll' \
  -H 'Content-Type: application/json' \
  -d '{"product_group":"SwapU","instrumentID":"BTCUSDT","asset":"USDT"}' 2>&1

echo ""
echo "[4] ping 测试"
echo "----------------------------------------"
ping -c 3 uuapi.rerrkvifj.com

echo ""
echo "[5] traceroute 测试"
echo "----------------------------------------"
traceroute uuapi.rerrkvifj.com 2>/dev/null || tracepath uuapi.rerrkvifj.com 2>/dev/null || echo "traceroute 不可用"

echo ""
echo "=========================================="
echo "诊断完成"
echo "=========================================="
