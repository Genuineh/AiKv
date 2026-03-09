#!/usr/bin/env node
/**
 * Tempo TraceQL 查询重写代理
 *
 * Grafana 12 Trace Exploration 会发送包含 nestedSetParent<0 的 TraceQL，
 * OSS Tempo 不支持该字段导致 400/500。本代理在转发前去掉该条件，使面板可正常显示。
 *
 * 用法: TEMPO_UPSTREAM=http://tempo:3200 node tempo-query-proxy.js
 * 监听: 0.0.0.0:3201
 */

const http = require('http');
const url = require('url');

const UPSTREAM = process.env.TEMPO_UPSTREAM || 'http://tempo:3200';
const PORT = parseInt(process.env.PORT || '3201', 10);
const DEBUG = process.env.DEBUG === '1';

function rewriteTraceQLQuery(raw) {
  if (!raw || typeof raw !== 'string') return raw;
  let q = raw;
  q = q.replace(/nestedSetParent\s*<\s*0\s*&&\s*/g, '');
  q = q.replace(/\s*&&\s*true\s*&&\s*/g, ' && ');
  q = q.replace(/^\s*&&\s*/, '');
  return q;
}

function rewriteQueryString(search) {
  if (!search || !search.startsWith('?')) return search;
  const params = new url.URLSearchParams(search.slice(1));
  const q = params.get('q');
  if (q && q.includes('nestedSetParent')) {
    params.set('q', rewriteTraceQLQuery(q));
    if (DEBUG) console.log('rewrite URL q:', q.slice(0, 80) + '... -> ', params.get('q').slice(0, 80) + '...');
    return '?' + params.toString();
  }
  return search;
}

function rewriteBody(contentType, body) {
  if (!body || !body.includes('nestedSetParent')) return body;
  let out = body;
  if (contentType && contentType.includes('application/x-www-form-urlencoded')) {
    const params = new url.URLSearchParams(body);
    const q = params.get('q');
    if (q) {
      params.set('q', rewriteTraceQLQuery(q));
      out = params.toString();
      if (DEBUG) console.log('rewrite POST form q');
    }
  } else if (contentType && contentType.includes('application/json')) {
    try {
      const j = JSON.parse(body);
      if (j.query && typeof j.query === 'string') {
        j.query = rewriteTraceQLQuery(j.query);
        out = JSON.stringify(j);
        if (DEBUG) console.log('rewrite POST JSON query');
      } else if (j.q && typeof j.q === 'string') {
        j.q = rewriteTraceQLQuery(j.q);
        out = JSON.stringify(j);
        if (DEBUG) console.log('rewrite POST JSON q');
      }
    } catch (_) {}
  }
  return out;
}

function doProxy(clientReq, clientRes, path, body) {
  const u = url.parse(UPSTREAM);
  const headers = { ...clientReq.headers };
  headers.host = u.host || u.hostname + (u.port ? ':' + u.port : '');
  if (body != null) {
    const buf = Buffer.from(body, typeof body === 'string' ? 'utf8' : undefined);
    headers['content-length'] = buf.length;
  }
  const opts = {
    hostname: u.hostname,
    port: parseInt(u.port, 10) || 80,
    path,
    method: clientReq.method,
    headers,
  };
  const proxy = http.request(opts, (upRes) => {
    clientRes.writeHead(upRes.statusCode, upRes.headers);
    upRes.pipe(clientRes);
  });
  proxy.on('error', (err) => {
    clientRes.writeHead(502, { 'Content-Type': 'text/plain' });
    clientRes.end('Bad Gateway: ' + err.message);
  });
  if (body != null) {
    proxy.write(Buffer.isBuffer(body) ? body : Buffer.from(body, 'utf8'));
    proxy.end();
  } else {
    clientReq.pipe(proxy);
  }
}

const server = http.createServer((clientReq, clientRes) => {
  const parsed = url.parse(clientReq.url, true);
  const newSearch = rewriteQueryString(parsed.search || '');
  const path = (parsed.pathname || '/') + newSearch;
  const qParam = parsed.query && parsed.query.q;
  const rewritten = qParam && qParam.includes('nestedSetParent');
  if (parsed.pathname && parsed.pathname.includes('/api/metrics/')) {
    console.log(clientReq.method, parsed.pathname, rewritten ? '(nestedSetParent rewritten)' : '');
  }

  const hasBody = (clientReq.method === 'POST' || clientReq.method === 'PUT') &&
    clientReq.headers['content-length'] && parseInt(clientReq.headers['content-length'], 10) > 0;

  if (!hasBody) {
    doProxy(clientReq, clientRes, path, null);
    return;
  }

  const chunks = [];
  clientReq.on('data', (chunk) => chunks.push(chunk));
  clientReq.on('end', () => {
    const raw = Buffer.concat(chunks).toString('utf8');
    const contentType = clientReq.headers['content-type'] || '';
    const body = rewriteBody(contentType, raw);
    doProxy(clientReq, clientRes, path, body);
  });
});

server.listen(PORT, '0.0.0.0', () => {
  console.log('Tempo query proxy listening on 0.0.0.0:' + PORT + ', upstream=' + UPSTREAM);
});
