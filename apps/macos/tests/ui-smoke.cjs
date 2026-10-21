const { chromium } = require('playwright');
const http = require('http');
const fs = require('fs');
const path = require('path');
(async () => {
  const root = path.resolve(__dirname, '../ui');
  const server = http
    .createServer((req, res) => {
      const file = path.join(root, req.url === '/' ? 'index.html' : req.url);
      if (!fs.existsSync(file)) {
        res.statusCode = 404;
        res.end();
        return;
      }
      res.setHeader(
        'content-type',
        file.endsWith('.js') ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : 'text/html'
      );
      res.end(fs.readFileSync(file));
    })
    .listen(0, '127.0.0.1');
  await new Promise(r => server.on('listening', r));
  const browser = await chromium.launch({
    headless: true,
    executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE
  });
  const page = await browser.newPage({
    viewport: { width: 1080, height: 850 }
  });
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  await page.addInitScript(() => {
    window.calls = [];
    window.rejectCatalog = false;
    const host = 'a'.repeat(64),
      peer = 'b'.repeat(64);
    window.fixture = {
      api_version: 1,
      host_name: 'Local desktop',
      relay: 'connected',
      host: {
        host,
        revision: 7,
        clients: [{ peer, name: 'Remote host' }],
        services: [
          {
            id: 'web',
            display_name: 'Website',
            kind: 'http',
            backend: '127.0.0.1:8080',
            incarnation: 'c'.repeat(64),
            allow: [peer]
          }
        ]
      },
      peer_pins: [peer],
      invitations: [],
      connected_peers: { [peer]: 1 },
      recent_peers: {},
      peer_telemetry: { [peer]: { path: 'direct', rtt_ms: 12 } }
    };
    window.__TAURI__ = {
      core: {
        invoke: async (command, args) => {
          window.calls.push({ command, ...args });
          if (command === 'legacy_peer_names') return {};
          if (command === 'build_identity') return 'test-build';
          const p = args?.payload;
          if (command === 'desktop_data') {
            if (p.op === 'preferences') return { names: {}, order: {} };
            if (p.op === 'operations') return [];
            if (p.op === 'targets') return [{ host }];
            return null;
          }
          if (command === 'node_request') {
            if (p.op === 'status') return structuredClone(window.fixture);
            if (p.op === 'catalog' && window.rejectCatalog) throw Error('Host unavailable');
            if (p.op === 'catalog')
              return {
                host: peer,
                services: [
                  {
                    id: 'llama',
                    display_name: 'Llama',
                    kind: 'http',
                    incarnation: 'd'.repeat(64)
                  }
                ]
              };
            if (p.op === 'gateway_settings')
              return {
                settings: {
                  registry_port: 21543,
                  allowed_origins: ['https://example.com'],
                  service_ports: []
                }
              };
            return {};
          }
        }
      }
    };
  });
  await page.goto(`http://127.0.0.1:${server.address().port}`);
  await page.getByRole('heading', { name: 'Remote host', exact: true }).waitFor();
  await page.getByRole('button', { name: 'Access settings', exact: true }).click();
  await page.getByLabel('Registry port', { exact: true }).fill('9001');
  await page.getByRole('button', { name: 'Save settings', exact: true }).click();
  await page.waitForFunction(() => window.calls.some(c => c.payload?.op === 'configure_access'));
  await page.getByRole('button', { name: 'My services', exact: true }).click();
  await page.getByRole('button', { name: 'Edit', exact: true }).click();
  await page.getByLabel('Display name', { exact: true }).fill('Renamed web');
  await page.getByRole('button', { name: 'Save service', exact: true }).click();
  await page.waitForFunction(() =>
    window.calls.some(c => c.payload?.operation?.display_name === 'Renamed web')
  );
  await page.getByRole('button', { name: 'Enable public access…', exact: true }).click();
  await page.getByRole('button', { name: 'Confirm', exact: true }).click();
  await page.waitForFunction(() => window.calls.some(c => c.payload?.op === 'configure_public'));
  await page.getByRole('button', { name: 'Owner operations', exact: true }).click();
  await page.getByRole('button', { name: 'Add client across hosts…', exact: true }).click();
  await page.getByLabel('Client public ID', { exact: true }).fill('f'.repeat(64));
  await page.getByLabel('Client name', { exact: true }).fill('New client');
  await page.locator('#dialog-body input[type=checkbox]').check();
  await page.getByRole('button', { name: 'Review operation', exact: true }).click();
  await page.getByRole('button', { name: 'Confirm operation', exact: true }).click();
  await page.waitForFunction(() => window.calls.some(c => c.payload?.op === 'start'));
  await page.getByRole('button', { name: 'Services', exact: true }).click();
  await page.evaluate(() => {
    window.rejectCatalog = true;
  });
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await page.getByText('Error: Host unavailable', { exact: true }).waitFor();
  if (!(await page.getByRole('button', { name: /Llama/ }).isDisabled()))
    throw Error('Stale catalog allowed Open');
  await page.getByRole('button', { name: 'Rename', exact: true }).click();
  await page.getByLabel('Private display name', { exact: true }).fill('Renamed peer');
  await page.getByRole('button', { name: 'Save name', exact: true }).click();
  await page.getByRole('heading', { name: 'Renamed peer', exact: true }).waitFor();
  // Approval must preserve the request and revision captured when its dialog opens.
  await page.evaluate(() => {
    const peer = 'b'.repeat(64);
    window.fixture.invitations = [
      { invitation: 'e'.repeat(64), peer, name: 'Test phone', state: 'pending' }
    ];
    window.fixture.host.services.push({
      id: 'beta',
      display_name: 'Beta',
      kind: 'http',
      backend: '127.0.0.1:8081',
      incarnation: 'f'.repeat(64),
      allow: []
    });
  });
  await page.getByRole('button', { name: /^Pairing requests/ }).click();
  await page.getByRole('button', { name: 'Approve…', exact: true }).waitFor();
  await page.getByRole('button', { name: 'Approve…', exact: true }).click();
  const existingGrant = page.getByLabel('Website', { exact: true });
  if (!(await existingGrant.isChecked()) || !(await existingGrant.isDisabled()))
    throw Error('Existing grant not preserved');
  await page.getByLabel('Beta', { exact: true }).check();
  await page.evaluate(() => {
    window.fixture.host.revision = 8;
    window.fixture.invitations = [];
  });
  await page.waitForTimeout(3100);
  await page.getByRole('button', { name: 'Approve device', exact: true }).click();
  const approval = await page.evaluate(
    () =>
      window.calls.find(c => c.payload?.operation?.op === 'approve_invitation')?.payload.operation
  );
  if (
    approval?.expected_revision !== 7 ||
    approval.invitation !== 'e'.repeat(64) ||
    JSON.stringify(approval.services) !== JSON.stringify(['web', 'beta'])
  )
    throw Error('Approval lost captured request/revision or selected grants');
  // Exercise keyboard ordering; service cards cannot move between machine groups.
  await page.getByRole('button', { name: 'Services', exact: true }).click();
  await page
    .getByRole('button', { name: 'Reorder web; use arrow keys to move', exact: true })
    .press('ArrowRight');
  const order = await page.evaluate(
    () => JSON.parse(localStorage.getItem('cardOrder'))['services/' + 'a'.repeat(64)]
  );
  if (JSON.stringify(order) !== JSON.stringify(['beta', 'web']))
    throw Error('Service ordering not persisted');
  if (process.env.WISPNET_UI_SCREENSHOT)
    await page.screenshot({
      path: process.env.WISPNET_UI_SCREENSHOT,
      fullPage: true
    });
  console.log(
    JSON.stringify({
      errors,
      calls: await page.evaluate(() =>
        window.calls.filter(c =>
          ['configure_access', 'mutate', 'configure_public', 'start'].includes(c.payload?.op)
        )
      )
    })
  );
  await browser.close();
  server.close();
  if (errors.length) process.exitCode = 1;
})().catch(e => {
  console.error(e);
  process.exit(1);
});
