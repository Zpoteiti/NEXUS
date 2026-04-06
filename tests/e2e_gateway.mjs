// E2E test for nexus-gateway bridge
// Tests that messages flow: browser → nexus-gateway → nexus-server
//              and:         nexus-server → nexus-gateway → browser
// Run with: NEXUS_GATEWAY_TOKEN=dev-token node tests/e2e_gateway.mjs

import { WebSocket } from 'ws';

const GATEWAY_URL = process.env.GATEWAY_URL || 'ws://localhost:9090';
const GATEWAY_TOKEN = process.env.NEXUS_GATEWAY_TOKEN || 'dev-token';

function wsConnect(url, timeoutMs = 3000) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url);
    const timer = setTimeout(() => {
      ws.terminate();
      reject(new Error(`connect timeout: ${url}`));
    }, timeoutMs);
    ws.on('open', () => { clearTimeout(timer); resolve(ws); });
    ws.on('error', (e) => { clearTimeout(timer); reject(e); });
  });
}

function waitMessage(ws, timeoutMs = 3000) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('message timeout')), timeoutMs);
    ws.once('message', (data) => {
      clearTimeout(timer);
      resolve(JSON.parse(data.toString()));
    });
  });
}

function assert(condition, msg) {
  if (!condition) {
    console.error(`  FAIL: ${msg}`);
    process.exit(1);
  }
  console.log(`  ✓ ${msg}`);
}

async function runTests() {
  console.log('=== nexus-gateway E2E Test ===\n');

  // --- Test 1: nexus auth flow ---
  console.log('Test 1: nexus auth handshake');

  const nexusWs = await wsConnect(`${GATEWAY_URL}/ws/nexus`);
  nexusWs.send(JSON.stringify({ type: 'auth', token: GATEWAY_TOKEN }));
  const authResp = await waitMessage(nexusWs);
  assert(authResp.type === 'auth_ok', `auth response is auth_ok (got: ${authResp.type})`);

  // --- Test 2: bad auth is rejected ---
  console.log('\nTest 2: bad token is rejected');
  const badWs = await wsConnect(`${GATEWAY_URL}/ws/nexus`);
  badWs.send(JSON.stringify({ type: 'auth', token: 'wrong-token' }));
  const badResp = await waitMessage(badWs);
  assert(badResp.type === 'auth_fail', `bad token returns auth_fail (got: ${badResp.type})`);
  badWs.terminate();

  // --- Test 3: browser message forwarded to nexus ---
  console.log('\nTest 3: browser → nexus-gateway → nexus-server');
  const browserWs = await wsConnect(`${GATEWAY_URL}/ws/chat`);
  browserWs.send(JSON.stringify({ type: 'message', content: 'hello from browser' }));

  const inbound = await waitMessage(nexusWs);
  assert(inbound.type === 'message', `nexus receives message type (got: ${inbound.type})`);
  assert(inbound.content === 'hello from browser', `content matches`);
  assert(typeof inbound.chat_id === 'string' && inbound.chat_id.length > 0, `chat_id is a non-empty string`);
  assert(typeof inbound.sender_id === 'string', `sender_id present`);
  const chatId = inbound.chat_id;

  // --- Test 4: nexus response routed back to browser ---
  console.log('\nTest 4: nexus-server → nexus-gateway → browser');
  nexusWs.send(JSON.stringify({ type: 'send', chat_id: chatId, content: 'reply from nexus' }));

  const outbound = await waitMessage(browserWs);
  assert(outbound.type === 'message', `browser receives message type (got: ${outbound.type})`);
  assert(outbound.content === 'reply from nexus', `content matches`);

  // --- Test 5: unknown chat_id is silently dropped (no crash) ---
  console.log('\nTest 5: send to unknown chat_id is silently dropped');
  nexusWs.send(JSON.stringify({ type: 'send', chat_id: 'nonexistent-id', content: 'drop me' }));
  // Give server 200ms to process — if it crashes, subsequent tests would fail
  await new Promise(r => setTimeout(r, 200));

  // Verify nexus WS is still alive by sending a known message
  const browserWs2 = await wsConnect(`${GATEWAY_URL}/ws/chat`);
  browserWs2.send(JSON.stringify({ type: 'message', content: 'still alive?' }));
  const probe = await waitMessage(nexusWs);
  assert(probe.type === 'message', `server still alive after unknown chat_id (got: ${probe.type})`);
  assert(probe.content === 'still alive?', `probe content matches`);
  browserWs2.terminate();

  // Cleanup
  nexusWs.terminate();
  browserWs.terminate();

  console.log('\n✅ All E2E tests passed!\n');
}

runTests().catch(e => {
  console.error('\n❌ E2E test failed:', e.message);
  process.exit(1);
});
