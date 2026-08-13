#!/bin/bash
# 带认证的直连测试

echo "=========================================="
echo "带认证的直连测试"
echo "=========================================="

# 认证信息
SECRET="23bec4f8489109e112812c2c2c7c31b3"
UID="LBA8G85737"
TOKEN="0688c69dd06a41f38c482e0f46719ed8"
DEVICE_ID="hZlegXdOAxOsNqUVl7oL8p8lwE3dIeqQ"
USER_AGENT="Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36"
VERSION_CODE="20251120"
CHANNEL="WEB"
CLIENT_TYPE="WEB"
METHOD="POST"
PATH="/cfd/agg/v1/sendQryAll"
TIMESTAMP=$(date +%s%3)

# 构建签名字符串: [METHOD][PATH][TIMESTAMP][USER_AGENT][VERSION_CODE][CHANNEL][CLIENT_TYPE][DEVICE_ID]
SIGN_STRING="${METHOD}${PATH}${TIMESTAMP}${USER_AGENT}${VERSION_CODE}${CHANNEL}${CLIENT_TYPE}${DEVICE_ID}"

echo "Sign String (first 200 chars): ${SIGN_STRING:0:200}..."
echo ""

# 计算 HMAC-SHA256
SIGNATURE=$(echo -n "$SIGN_STRING" | openssl dgst -sha256 -hmac "$SECRET" -binary 2>/dev/null | base64)

echo "Timestamp: $TIMESTAMP"
echo "Signature: $SIGNATURE"

echo ""
echo "[1] 带认证 headers 的测试"
echo "----------------------------------------"
curl -s --noproxy '*' -X POST 'https://uuapi.rerrkvifj.com/cfd/agg/v1/sendQryAll' \
  -H 'Content-Type: application/json' \
  -H "ex-uid: $UID" \
  -H "ex-token: $TOKEN" \
  -H "ex-device-id: $DEVICE_ID" \
  -H "ex-client-type: $CLIENT_TYPE" \
  -H "ex-client-channel: $CHANNEL" \
  -H "ex-client-version-code: $VERSION_CODE" \
  -H "ex-client-source: WEB" \
  -H "User-Agent: $USER_AGENT" \
  -H "ex-language: zh-TW" \
  -H "ex-timestamp: $TIMESTAMP" \
  -H "ex-signature: $SIGNATURE" \
  -H "businessversioncode: 202" \
  -H "versionflage: true" \
  -d '{"product_group":"SwapU","instrumentID":"BTCUSDT","asset":"USDT"}'

echo ""
echo ""
echo "=========================================="
echo "测试完成"
echo "=========================================="
