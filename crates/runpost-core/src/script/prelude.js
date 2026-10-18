// Preludio de los scripts (sección 10): subconjunto de `pm.*` compatible con Postman.
// Se evalúa en un contexto QuickJS nuevo antes de cada script. Rust deja antes:
//   __rp_input            JSON con fase, variables, petición, respuesta e info.
//   __rp_digest(alg, data, enc), __rp_hmac(alg, key, data, enc), __rp_base64(mode, text),
//   __rp_send(json)       funciones nativas (hash, base64 y `pm.sendRequest`).
// y después llama a __rp_next_timer(), __rp_run_timers(), __rp_script_error(e) y __rp_finish().
(function () {
  'use strict';
  const g = globalThis;
  const input = JSON.parse(g.__rp_input);
  const native = {
    digest: g.__rp_digest,
    hmac: g.__rp_hmac,
    base64: g.__rp_base64,
    send: g.__rp_send,
  };
  for (const name of ['__rp_input', '__rp_digest', '__rp_hmac', '__rp_base64', '__rp_send']) delete g[name];

  const phase = input.phase; // 'prerequest' | 'test'
  const started = Date.now();
  const MAX_CONSOLE = 500;
  const MAX_LINE = 2000;
  const MAX_SENDS = 10;
  const out = { tests: [], console: [], changes: [], requestHeaders: null, skip: false };
  let sends = 0;

  // ---------- formato (como chai/loupe, sin colores) ----------

  function typeOf(value) {
    if (value === null) return 'null';
    if (Array.isArray(value)) return 'array';
    if (value instanceof RegExp) return 'regexp';
    if (value instanceof Date) return 'date';
    if (value instanceof Error) return 'error';
    if (value instanceof Map) return 'map';
    if (value instanceof Set) return 'set';
    if (typeof Promise !== 'undefined' && value instanceof Promise) return 'promise';
    return typeof value;
  }

  function inspect(value, seen) {
    seen = seen || [];
    const type = typeOf(value);
    switch (type) {
      case 'null':
      case 'undefined':
      case 'boolean':
        return String(value);
      case 'number':
        return Object.is(value, -0) ? '-0' : String(value);
      case 'bigint':
        return String(value) + 'n';
      case 'string':
        return "'" + value.replace(/\\/g, '\\\\').replace(/'/g, "\\'").replace(/\n/g, '\\n') + "'";
      case 'symbol':
        return value.toString();
      case 'function':
        return value.name ? '[Function ' + value.name + ']' : '[Function]';
      case 'regexp':
        return String(value);
      case 'date':
        return isNaN(value.getTime()) ? 'Invalid Date' : value.toISOString();
      case 'error':
        return value.name + ': ' + value.message;
    }
    if (seen.indexOf(value) >= 0) return '[Circular]';
    const inner = seen.concat([value]);
    if (type === 'array') {
      return value.length ? '[ ' + value.map((v) => inspect(v, inner)).join(', ') + ' ]' : '[]';
    }
    if (type === 'map') return 'Map{' + Array.from(value).map(([k, v]) => inspect(k, inner) + ' => ' + inspect(v, inner)).join(', ') + '}';
    if (type === 'set') return 'Set{' + Array.from(value).map((v) => inspect(v, inner)).join(', ') + '}';
    const keys = Object.keys(value);
    if (!keys.length) return '{}';
    return '{ ' + keys.map((k) => (/^[A-Za-z_$][\w$]*$/.test(k) ? k : inspect(k)) + ': ' + inspect(value[k], inner)).join(', ') + ' }';
  }

  // Igual que objDisplay de chai: objetos y arrays largos se resumen.
  function display(value) {
    const text = inspect(value);
    if (text.length < 40) return text;
    const type = typeOf(value);
    if (type === 'function') return value.name ? '[Function: ' + value.name + ']' : '[Function]';
    if (type === 'array') return '[ Array(' + value.length + ') ]';
    if (type === 'object') {
      const keys = Object.keys(value);
      return '{ Object (' + (keys.length > 2 ? keys.slice(0, 2).join(', ') + ', ...' : keys.join(', ')) + ') }';
    }
    return text;
  }

  function consoleText(value) {
    if (typeof value === 'string') return value;
    return inspect(value);
  }

  function describe(error) {
    if (error && typeof error === 'object' && 'message' in error) {
      const name = error.name || 'Error';
      return error.message ? name + ': ' + error.message : name;
    }
    return 'Error: ' + String(error);
  }

  function asText(value) {
    if (value === undefined || value === null) return '';
    if (typeof value === 'object') {
      try {
        return JSON.stringify(value);
      } catch (e) {
        return String(value);
      }
    }
    return String(value);
  }

  // ---------- consola ----------

  function log(level) {
    return function () {
      if (out.console.length >= MAX_CONSOLE) return;
      let line = Array.prototype.map.call(arguments, consoleText).join(' ');
      if (line.length > MAX_LINE) line = line.slice(0, MAX_LINE) + '…';
      out.console.push(level === 'log' ? line : '[' + level + '] ' + line);
    };
  }
  g.console = { log: log('log'), info: log('info'), warn: log('warn'), error: log('error'), debug: log('debug') };

  // ---------- temporizadores (la cola la vacía Rust tras el script) ----------

  let timerSeq = 0;
  const timers = [];
  function elapsed() {
    return Date.now() - started;
  }
  function addTimer(fn, ms, args, repeat) {
    if (typeof fn !== 'function') throw new TypeError('callback must be a function');
    const id = ++timerSeq;
    const delay = Math.max(0, Number(ms) || 0);
    timers.push({ id: id, due: elapsed() + delay, delay: delay, fn: fn, args: args, repeat: repeat });
    return id;
  }
  function clearTimer(id) {
    const index = timers.findIndex((t) => t.id === id);
    if (index >= 0) timers.splice(index, 1);
  }
  g.setTimeout = (fn, ms, ...args) => addTimer(fn, ms, args, false);
  g.setInterval = (fn, ms, ...args) => addTimer(fn, ms, args, true);
  g.clearTimeout = clearTimer;
  g.clearInterval = clearTimer;
  g.__rp_next_timer = function () {
    if (!timers.length) return -1;
    let due = Infinity;
    for (const t of timers) due = Math.min(due, t.due);
    return Math.max(0, due - elapsed());
  };
  g.__rp_run_timers = function () {
    const now = elapsed();
    const ready = timers.filter((t) => t.due <= now).sort((a, b) => a.due - b.due || a.id - b.id);
    for (const t of ready) {
      const index = timers.indexOf(t);
      if (index < 0) continue; // cancelado por otro temporizador
      if (t.repeat) t.due = elapsed() + Math.max(1, t.delay);
      else timers.splice(index, 1);
      try {
        t.fn.apply(undefined, t.args);
      } catch (e) {
        g.__rp_script_error(e);
      }
    }
  };
  g.__rp_script_error = function (error) {
    out.tests.push({ name: '<script error>', passed: false, msg: describe(error) });
  };

  // ---------- base64 y criptografía nativas ----------

  g.btoa = (text) => native.base64('btoa', String(text));
  g.atob = (text) => native.base64('atob', String(text));
  const base64 = (text) => native.base64('encode', asText(text));
  base64.decode = (text) => native.base64('decode', String(text));
  g.runpost = Object.freeze({
    crypto: Object.freeze({
      sha256: (text, encoding) => native.digest('sha256', asText(text), encoding || 'hex'),
      sha1: (text, encoding) => native.digest('sha1', asText(text), encoding || 'hex'),
      sha512: (text, encoding) => native.digest('sha512', asText(text), encoding || 'hex'),
      hmac: (key, text, algorithm, encoding) => native.hmac(algorithm || 'sha256', asText(key), asText(text), encoding || 'hex'),
      base64: base64,
    }),
  });

  // APIs de Postman que RunPost no ofrece: error claro en vez de "undefined is not a function".
  function unsupported(name, hint) {
    return new Proxy(function () {}, {
      get(_, prop) {
        if (prop === Symbol.toPrimitive || prop === 'toString') return () => '[unsupported ' + name + ']';
        throw new Error(name + ' is not supported in RunPost scripts' + (hint ? ': ' + hint : ''));
      },
      apply() {
        throw new Error(name + ' is not supported in RunPost scripts' + (hint ? ': ' + hint : ''));
      },
    });
  }
  g.require = function (name) {
    throw new Error('require(' + inspect(name) + ') is not supported in RunPost scripts');
  };
  g.CryptoJS = unsupported('CryptoJS', 'use runpost.crypto.sha256 / hmac / base64');
  g.postman = unsupported('postman.* (legacy API)', 'use pm.*');

  // ---------- variables ----------

  function record(scope, op, key, value) {
    out.changes.push(op === 'set' ? { scope: scope, op: op, key: key, value: value } : { scope: scope, op: op, key: key });
  }

  const DYNAMIC = {
    $guid: () =>
      'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (c) => {
        const r = (Math.random() * 16) | 0;
        return (c === 'x' ? r : (r & 0x3) | 0x8).toString(16);
      }),
    $timestamp: () => String(Math.floor(Date.now() / 1000)),
    $isoTimestamp: () => new Date().toISOString(),
    $randomInt: () => String(Math.floor(Math.random() * 1001)),
  };

  function replaceIn(template, lookup) {
    if (typeof template !== 'string') return template;
    const expand = (text, stack) =>
      text.replace(/\{\{([^{}]+?)\}\}/g, (match, raw) => {
        const name = raw.trim();
        if (!name || stack.indexOf(name) >= 0 || stack.length >= 10) return match;
        let value = lookup(name);
        if (value === undefined && Object.prototype.hasOwnProperty.call(DYNAMIC, name)) value = DYNAMIC[name]();
        if (value === undefined) return match;
        return expand(asText(value), stack.concat([name]));
      });
    return expand(template, []);
  }

  function makeScope(scope, list, name) {
    const values = new Map();
    for (const item of list || []) values.set(item.key, item.value);
    const api = {
      get(key) {
        key = String(key);
        return values.has(key) ? values.get(key) : undefined;
      },
      set(key, value) {
        key = String(key);
        values.set(key, value);
        record(scope, 'set', key, asText(value));
      },
      unset(key) {
        key = String(key);
        if (values.delete(key)) record(scope, 'unset', key);
      },
      has(key) {
        return values.has(String(key));
      },
      clear() {
        for (const key of Array.from(values.keys())) api.unset(key);
      },
      toObject() {
        const object = {};
        values.forEach((value, key) => (object[key] = value));
        return object;
      },
      replaceIn(text) {
        return replaceIn(text, (key) => (values.has(key) ? values.get(key) : undefined));
      },
    };
    if (name !== undefined) api.name = name;
    return { api: api, values: values };
  }

  const vars = input.vars;
  const environment = makeScope('environment', vars.environment ? vars.environment.values : [], vars.environment ? vars.environment.name : undefined);
  const globals = makeScope('globals', vars.globals);
  const collection = makeScope('collection', vars.collection);
  const local = new Map(Object.entries(vars.local || {}));
  const iteration = new Map(Object.entries(vars.iteration || {}));
  // Orden de pm.variables (Postman): local > datos de iteración > entorno > colección > globales.
  const layers = [local, iteration, environment.values, collection.values, globals.values];
  const lookup = (key) => {
    for (const layer of layers) if (layer.has(key)) return layer.get(key);
    return undefined;
  };

  // ---------- cabeceras, URL y cuerpo ----------

  function toHeader(header) {
    if (typeof header === 'string') {
      const at = header.indexOf(':');
      return at < 0 ? { key: header.trim(), value: '', disabled: false } : { key: header.slice(0, at).trim(), value: header.slice(at + 1).trim(), disabled: false };
    }
    if (!header || header.key === undefined) throw new TypeError('header must be {key, value} or "Key: value"');
    return { key: String(header.key), value: asText(header.value), disabled: !!header.disabled };
  }

  function headerList(items, mutable) {
    const list = (items || []).map(toHeader);
    const same = (name) => (h) => h.key.toLowerCase() === String(name).toLowerCase();
    const active = () => list.filter((h) => !h.disabled);
    const changed = () => {
      out.requestHeaders = list.map((h) => ({ key: h.key, value: h.value, disabled: h.disabled }));
    };
    const api = {
      get(name) {
        const found = active().find(same(name));
        return found ? found.value : undefined;
      },
      has(name, value) {
        const found = active().find(same(name));
        return !!found && (value === undefined || found.value === String(value));
      },
      one(name) {
        const found = active().find(same(name));
        return found ? { key: found.key, value: found.value } : undefined;
      },
      all() {
        return active().map((h) => ({ key: h.key, value: h.value }));
      },
      toObject() {
        const object = {};
        for (const h of active()) object[h.key] = h.value;
        return object;
      },
      each(fn) {
        api.all().forEach(fn);
      },
      map(fn) {
        return api.all().map(fn);
      },
      filter(fn) {
        return api.all().filter(fn);
      },
      count() {
        return active().length;
      },
      toJSON() {
        return api.all();
      },
      toString() {
        return api.all().map((h) => h.key + ': ' + h.value).join('\n');
      },
    };
    if (mutable) {
      api.add = (header) => {
        list.push(toHeader(header));
        changed();
      };
      api.upsert = (header) => {
        const next = toHeader(header);
        const found = list.find(same(next.key));
        if (found) {
          found.value = next.value;
          found.disabled = false;
        } else list.push(next);
        changed();
      };
      api.remove = (predicate) => {
        const test = typeof predicate === 'function' ? predicate : same(predicate);
        for (let i = list.length - 1; i >= 0; i -= 1) if (test(list[i])) list.splice(i, 1);
        changed();
      };
      api.clear = () => {
        list.length = 0;
        changed();
      };
    }
    return api;
  }

  function makeUrl(raw) {
    const text = String(raw || '');
    const match = text.match(/^(?:([a-z][a-z0-9+.-]*):\/\/)?([^/?#]*)([^?#]*)(?:\?([^#]*))?/i) || [];
    const authority = match[2] || '';
    const hostPort = authority.slice(authority.lastIndexOf('@') + 1);
    const portMatch = hostPort.match(/:(\d+)$/);
    const host = portMatch ? hostPort.slice(0, -portMatch[0].length) : hostPort;
    const pairs = (match[4] || '')
      .split('&')
      .filter(Boolean)
      .map((part) => {
        const at = part.indexOf('=');
        return at < 0 ? { key: part, value: null } : { key: part.slice(0, at), value: part.slice(at + 1) };
      });
    return {
      protocol: match[1] || 'http',
      host: host ? host.split('.') : [],
      port: portMatch ? portMatch[1] : undefined,
      path: (match[3] || '').split('/').filter(Boolean),
      query: {
        get: (key) => {
          const found = pairs.find((p) => p.key === key);
          return found ? found.value : undefined;
        },
        has: (key) => pairs.some((p) => p.key === key),
        all: () => pairs.map((p) => ({ key: p.key, value: p.value })),
        toObject: () => Object.fromEntries(pairs.map((p) => [p.key, p.value])),
        count: () => pairs.length,
      },
      getHost: () => host,
      getPath: () => match[3] || '/',
      getQueryString: () => match[4] || '',
      getPathWithQuery: () => (match[3] || '/') + (match[4] ? '?' + match[4] : ''),
      toString: () => text,
      toJSON: () => text,
    };
  }

  function makeBody(body) {
    if (!body) return undefined;
    const copy = JSON.parse(JSON.stringify(body));
    copy.toString = () => (copy.mode === 'raw' ? copy.raw || '' : JSON.stringify(copy[copy.mode] || ''));
    copy.isEmpty = () => !copy.mode || copy.mode === 'none';
    return copy;
  }

  // ---------- aserciones (subconjunto de chai) ----------

  class AssertionError extends Error {
    constructor(message) {
      super(message);
      this.name = 'AssertionError';
    }
  }

  function deepEqual(a, b, seen) {
    if (Object.is(a, b) || a === b) return true;
    if (typeof a !== 'object' || typeof b !== 'object' || a === null || b === null) return false;
    if (Object.getPrototypeOf(a) !== Object.getPrototypeOf(b)) return false;
    if (a instanceof Date) return a.getTime() === b.getTime();
    if (a instanceof RegExp) return String(a) === String(b);
    seen = seen || [];
    if (seen.some((pair) => pair[0] === a && pair[1] === b)) return true;
    seen = seen.concat([[a, b]]);
    if (a instanceof Map || a instanceof Set) {
      const left = Array.from(a);
      const right = Array.from(b);
      return left.length === right.length && left.every((v, i) => deepEqual(v, right[i], seen));
    }
    const keysA = Object.keys(a);
    const keysB = Object.keys(b);
    if (keysA.length !== keysB.length) return false;
    return keysA.every((k) => Object.prototype.hasOwnProperty.call(b, k) && deepEqual(a[k], b[k], seen));
  }

  function getPath(object, path) {
    const parts = String(path)
      .replace(/\[(\w+)\]/g, '.$1')
      .split('.')
      .filter(Boolean);
    let current = object;
    for (const part of parts) {
      if (current === null || current === undefined || !(part in Object(current))) return { exists: false, value: undefined };
      current = current[part];
    }
    return { exists: true, value: current };
  }

  function lengthOf(value) {
    if (value instanceof Map || value instanceof Set) return value.size;
    return value === null || value === undefined ? undefined : value.length;
  }

  function Assertion(value, message, flags) {
    this._obj = value;
    this._msg = message;
    this._flags = flags || {};
  }

  // Chainable de chai: se puede llamar (`.include(x)`) o seguir encadenando (`.include.members`).
  function chainable(assertion, call, flag) {
    if (flag) assertion._flags[flag] = true;
    const fn = function () {
      return call.apply(assertion, arguments);
    };
    return new Proxy(fn, {
      get(target, prop) {
        if (prop in assertion) {
          const value = assertion[prop];
          return typeof value === 'function' ? value.bind(assertion) : value;
        }
        return target[prop];
      },
    });
  }

  const proto = Assertion.prototype;
  proto.assert = function (ok, message, negated, expected, actual) {
    const negate = !!this._flags.not;
    if (negate ? !ok : ok) return this;
    const hasActual = arguments.length > 4;
    const text = (negate ? negated : message)
      .replace(/#\{this\}/g, () => display(this._obj))
      .replace(/#\{exp\}/g, () => display(expected))
      .replace(/#\{act\}/g, () => display(hasActual ? actual : this._obj));
    throw new AssertionError(this._msg ? this._msg + ': ' + text : text);
  };
  for (const word of ['to', 'be', 'been', 'is', 'that', 'which', 'and', 'has', 'have', 'with', 'at', 'of', 'same', 'but', 'does', 'still', 'also']) {
    Object.defineProperty(proto, word, { get() { return this; } });
  }
  for (const flag of ['not', 'deep', 'own', 'nested', 'any', 'all', 'ordered']) {
    Object.defineProperty(proto, flag, {
      get() {
        this._flags[flag] = flag === 'not' ? !this._flags.not : true;
        return this;
      },
    });
  }
  function terminal(name, check, message, negated) {
    Object.defineProperty(proto, name, {
      get() {
        return this.assert(check(this._obj), message, negated);
      },
    });
  }
  terminal('ok', (v) => !!v, 'expected #{this} to be truthy', 'expected #{this} to be falsy');
  terminal('true', (v) => v === true, 'expected #{this} to be true', 'expected #{this} to be false');
  terminal('false', (v) => v === false, 'expected #{this} to be false', 'expected #{this} to be true');
  terminal('null', (v) => v === null, 'expected #{this} to be null', 'expected #{this} not to be null');
  terminal('undefined', (v) => v === undefined, 'expected #{this} to be undefined', 'expected #{this} not to be undefined');
  terminal('NaN', (v) => typeof v === 'number' && isNaN(v), 'expected #{this} to be NaN', 'expected #{this} not to be NaN');
  terminal('exist', (v) => v !== null && v !== undefined, 'expected #{this} to exist', 'expected #{this} to not exist');
  terminal('finite', (v) => typeof v === 'number' && isFinite(v), 'expected #{this} to be a finite number', 'expected #{this} to not be a finite number');
  Object.defineProperty(proto, 'empty', {
    get() {
      const v = this._obj;
      const type = typeOf(v);
      const size = type === 'object' ? Object.keys(v).length : lengthOf(v);
      if (size === undefined) throw new TypeError('.empty was passed non-string primitive ' + display(v));
      return this.assert(size === 0, 'expected #{this} to be empty', 'expected #{this} not to be empty');
    },
  });

  function typeCheck(type) {
    type = String(type).toLowerCase();
    const article = 'aeiou'.indexOf(type.charAt(0)) >= 0 ? 'an ' : 'a ';
    return this.assert(typeOf(this._obj) === type, 'expected #{this} to be ' + article + type, 'expected #{this} not to be ' + article + type);
  }
  Object.defineProperty(proto, 'a', { get() { return chainable(this, typeCheck); } });
  Object.defineProperty(proto, 'an', { get() { return chainable(this, typeCheck); } });

  function equal(expected) {
    if (this._flags.deep) return eql.call(this, expected);
    if (this._flags.doLength) {
      const length = lengthOf(this._obj);
      return this.assert(length === expected, 'expected #{this} to have a length of #{exp} but got #{act}', 'expected #{this} to not have a length of #{act}', expected, length);
    }
    return this.assert(this._obj === expected, 'expected #{this} to equal #{exp}', 'expected #{this} to not equal #{exp}', expected);
  }
  function eql(expected) {
    return this.assert(deepEqual(this._obj, expected), 'expected #{this} to deeply equal #{exp}', 'expected #{this} to not deeply equal #{exp}', expected);
  }
  proto.equal = proto.equals = proto.eq = equal;
  proto.eql = proto.eqls = eql;

  function compare(name, check, message, negated) {
    return function (n) {
      if (this._flags.doLength) {
        const length = lengthOf(this._obj);
        return this.assert(check(length, n), 'expected #{this} to have a length ' + message + ' #{exp} but got #{act}', 'expected #{this} to not have a length ' + message + ' #{exp}', n, length);
      }
      return this.assert(check(this._obj, n), 'expected #{this} to be ' + message + ' #{exp}', 'expected #{this} to be ' + negated + ' #{exp}', n);
    };
  }
  proto.above = proto.gt = proto.greaterThan = compare('above', (a, b) => a > b, 'above', 'at most');
  proto.below = proto.lt = proto.lessThan = compare('below', (a, b) => a < b, 'below', 'at least');
  proto.least = proto.gte = proto.greaterThanOrEqual = compare('least', (a, b) => a >= b, 'at least', 'below');
  proto.most = proto.lte = proto.lessThanOrEqual = compare('most', (a, b) => a <= b, 'at most', 'above');
  proto.within = function (start, finish) {
    const value = this._flags.doLength ? lengthOf(this._obj) : this._obj;
    const range = start + '..' + finish;
    return this.assert(value >= start && value <= finish, 'expected #{this} to be within ' + range, 'expected #{this} to not be within ' + range);
  };
  proto.closeTo = proto.approximately = function (expected, delta) {
    return this.assert(Math.abs(this._obj - expected) <= delta, 'expected #{this} to be close to ' + expected + ' +/- ' + delta, 'expected #{this} not to be close to ' + expected + ' +/- ' + delta);
  };

  function lengthCall(n) {
    const length = lengthOf(this._obj);
    return this.assert(length === n, 'expected #{this} to have a length of #{exp} but got #{act}', 'expected #{this} to not have a length of #{act}', n, length);
  }
  for (const name of ['length', 'lengthOf']) {
    Object.defineProperty(proto, name, { get() { return chainable(this, lengthCall, 'doLength'); } });
  }

  function include(expected) {
    const value = this._obj;
    const type = typeOf(value);
    const deep = !!this._flags.deep;
    if (type === 'string') {
      return this.assert(value.indexOf(expected) >= 0, 'expected #{this} to include #{exp}', 'expected #{this} to not include #{exp}', expected);
    }
    if (type === 'array' || type === 'set') {
      const items = Array.from(value);
      const found = items.some((item) => (deep ? deepEqual(item, expected) : item === expected));
      return this.assert(found, 'expected #{this} to ' + (deep ? 'deep ' : '') + 'include #{exp}', 'expected #{this} to not ' + (deep ? 'deep ' : '') + 'include #{exp}', expected);
    }
    if (type === 'map') {
      const found = Array.from(value.values()).some((item) => (deep ? deepEqual(item, expected) : item === expected));
      return this.assert(found, 'expected #{this} to include #{exp}', 'expected #{this} to not include #{exp}', expected);
    }
    if (type === 'object' && expected && typeof expected === 'object') {
      let ok = true;
      for (const key of Object.keys(expected)) {
        const has = key in value && (deep ? deepEqual(value[key], expected[key]) : value[key] === expected[key]);
        if (!has) ok = false;
      }
      return this.assert(ok, 'expected #{this} to have ' + (deep ? 'deep ' : '') + 'properties #{exp}', 'expected #{this} to not have ' + (deep ? 'deep ' : '') + 'properties #{exp}', expected);
    }
    throw new TypeError('the given combination of arguments (' + typeOf(value) + ' and ' + typeOf(expected) + ') is invalid for this assertion');
  }
  for (const name of ['include', 'includes', 'contain', 'contains']) {
    Object.defineProperty(proto, name, { get() { return chainable(this, include, 'contains'); } });
  }

  proto.match = proto.matches = function (re) {
    return this.assert(re.test(this._obj), 'expected #{this} to match ' + re, 'expected #{this} not to match ' + re);
  };
  proto.string = function (text) {
    return this.assert(String(this._obj).indexOf(text) >= 0, 'expected #{this} to contain ' + display(text), 'expected #{this} to not contain ' + display(text));
  };
  proto.property = function (name, expected) {
    const value = this._obj;
    const kind = (this._flags.deep ? 'deep ' : '') + (this._flags.own ? 'own ' : '') + (this._flags.nested ? 'nested ' : '');
    let exists;
    let actual;
    if (this._flags.nested) {
      const found = getPath(value, name);
      exists = found.exists;
      actual = found.value;
    } else {
      exists = value !== null && value !== undefined && (this._flags.own ? Object.prototype.hasOwnProperty.call(value, name) : name in Object(value));
      actual = exists ? value[name] : undefined;
    }
    if (arguments.length > 1) {
      const same = exists && (this._flags.deep ? deepEqual(actual, expected) : actual === expected);
      this.assert(same, 'expected #{this} to have ' + kind + 'property ' + display(name) + ' of #{exp}, but got #{act}', 'expected #{this} to not have ' + kind + 'property ' + display(name) + ' of #{act}', expected, actual);
    } else {
      this.assert(exists, 'expected #{this} to have ' + kind + 'property ' + display(name), 'expected #{this} to not have ' + kind + 'property ' + display(name));
    }
    return new Assertion(actual, this._msg, { not: this._flags.not });
  };
  proto.ownProperty = proto.haveOwnProperty = function (name, expected) {
    this._flags.own = true;
    return arguments.length > 1 ? this.property(name, expected) : this.property(name);
  };
  proto.keys = proto.key = function () {
    let keys = Array.prototype.slice.call(arguments);
    if (keys.length === 1 && Array.isArray(keys[0])) keys = keys[0];
    else if (keys.length === 1 && keys[0] && typeof keys[0] === 'object') keys = Object.keys(keys[0]);
    const actual = this._obj instanceof Map ? Array.from(this._obj.keys()) : Object.keys(this._obj || {});
    const has = (k) => actual.indexOf(k) >= 0;
    let ok;
    if (this._flags.any) ok = keys.some(has);
    else ok = keys.every(has) && (this._flags.contains || keys.length === actual.length);
    const list = keys.map((k) => display(k)).join(', ');
    return this.assert(ok, 'expected #{this} to have keys ' + list, 'expected #{this} to not have keys ' + list);
  };
  proto.oneOf = function (list) {
    const found = list.some((item) => (this._flags.deep ? deepEqual(item, this._obj) : item === this._obj));
    return this.assert(found, 'expected #{this} to be one of #{exp}', 'expected #{this} to not be one of #{exp}', list);
  };
  proto.members = function (list) {
    const actual = Array.from(this._obj || []);
    const same = (a, b) => (this._flags.deep ? deepEqual(a, b) : a === b);
    const contained = list.every((item) => actual.some((a) => same(a, item)));
    const ok = this._flags.contains ? contained : contained && actual.length === list.length;
    const what = this._flags.contains ? 'be a superset of' : 'have the same members as';
    return this.assert(ok, 'expected #{this} to ' + what + ' #{exp}', 'expected #{this} to not ' + what + ' #{exp}', list);
  };
  proto.instanceOf = proto.instanceof = function (ctor) {
    const name = (ctor && ctor.name) || String(ctor);
    return this.assert(this._obj instanceof ctor, 'expected #{this} to be an instance of ' + name, 'expected #{this} to not be an instance of ' + name);
  };
  proto.satisfy = proto.satisfies = function (matcher) {
    return this.assert(!!matcher(this._obj), 'expected #{this} to satisfy ' + display(matcher), 'expected #{this} to not satisfy ' + display(matcher));
  };
  proto.throw = proto.throws = proto.Throw = function (expected) {
    let thrown = null;
    try {
      this._obj();
    } catch (e) {
      thrown = e || new Error(String(e));
    }
    let ok = thrown !== null;
    if (ok && expected !== undefined) {
      if (typeof expected === 'function') ok = thrown instanceof expected;
      else if (expected instanceof RegExp) ok = expected.test(thrown.message);
      else ok = String(thrown.message).indexOf(String(expected)) >= 0;
    }
    return this.assert(ok, 'expected #{this} to throw ' + (expected === undefined ? 'an error' : display(expected)), 'expected #{this} to not throw ' + (expected === undefined ? 'an error' : display(expected)));
  };

  // ---------- respuesta (pm.response y pm.sendRequest) ----------

  function statusClass(code) {
    return Math.floor(code / 100);
  }

  function makeResponse(raw) {
    const headers = headerList(raw.headers, false);
    const response = {
      code: raw.code,
      status: raw.status,
      responseTime: raw.responseTime,
      responseSize: raw.size,
      headers: headers,
      text: () => raw.body,
      json: () => {
        try {
          return JSON.parse(raw.body);
        } catch (e) {
          const error = new SyntaxError(e.message);
          error.name = 'JSONError';
          throw error;
        }
      },
      reason: () => raw.status,
      size: () => ({ body: raw.size, header: 0, total: raw.size }),
      toJSON: () => ({ code: raw.code, status: raw.status, header: headers.all(), responseTime: raw.responseTime }),
    };
    Object.defineProperty(response, 'to', { get: () => responseAssertion(response, false) });
    return response;
  }

  function responseAssertion(response, negate) {
    const fail = (ok, message, negated) => {
      if (negate ? ok : !ok) throw new AssertionError(negate ? negated : message);
    };
    const api = {};
    for (const word of ['to', 'be', 'have', 'and', 'a', 'an']) Object.defineProperty(api, word, { get: () => api });
    Object.defineProperty(api, 'not', { get: () => responseAssertion(response, !negate) });
    const codeClass = (name, expected, label) => {
      Object.defineProperty(api, name, {
        get() {
          const ok = typeof expected === 'function' ? expected(response.code) : response.code === expected;
          fail(ok, 'expected response code to be ' + label + ' but found ' + response.code, 'expected response code to not be ' + label + ' but found ' + response.code);
          return api;
        },
      });
    };
    codeClass('ok', (c) => statusClass(c) === 2, '2XX');
    codeClass('success', (c) => statusClass(c) === 2, '2XX');
    codeClass('info', (c) => statusClass(c) === 1, '1XX');
    codeClass('redirection', (c) => statusClass(c) === 3, '3XX');
    codeClass('clientError', (c) => statusClass(c) === 4, '4XX');
    codeClass('serverError', (c) => statusClass(c) === 5, '5XX');
    codeClass('error', (c) => statusClass(c) === 4 || statusClass(c) === 5, '4XX or 5XX');
    codeClass('accepted', 202, '202');
    codeClass('badRequest', 400, '400');
    codeClass('unauthorized', 401, '401');
    codeClass('forbidden', 403, '403');
    codeClass('notFound', 404, '404');
    codeClass('rateLimited', 429, '429');
    api.status = (expected) => {
      if (typeof expected === 'string') {
        fail(response.status === expected, 'expected response to have status reason ' + display(expected) + ' but got ' + display(response.status), 'expected response to not have status reason ' + display(response.status));
      } else {
        fail(response.code === expected, 'expected response to have status code ' + display(expected) + ' but got ' + display(response.code), 'expected response to not have status code ' + display(response.code));
      }
      return api;
    };
    api.header = (key, value) => {
      const actual = response.headers.get(key);
      if (value === undefined) {
        fail(actual !== undefined, 'expected response to have header with key ' + display(key), 'expected response to not have header with key ' + display(key));
      } else {
        fail(actual === String(value), 'expected ' + display(key) + ' response header to be ' + display(String(value)) + ' but got ' + display(actual), 'expected ' + display(key) + ' response header to not be ' + display(String(value)));
      }
      return api;
    };
    api.body = (expected) => {
      const text = response.text();
      if (expected === undefined) fail(text.length > 0, 'expected response to have content in body', 'expected response to not have content in body');
      else if (expected instanceof RegExp) fail(expected.test(text), 'expected response body text to match ' + expected, 'expected response body text to not match ' + expected);
      else if (typeof expected === 'string') fail(text === expected, 'expected response body to equal ' + display(expected) + ' but got ' + display(text), 'expected response body to not equal ' + display(expected));
      else fail(deepEqual(safeJson(), expected), 'expected response body json to equal ' + display(expected), 'expected response body json to not equal ' + display(expected));
      return api;
    };
    const safeJson = () => {
      try {
        return JSON.parse(response.text());
      } catch (e) {
        return undefined;
      }
    };
    Object.defineProperty(api, 'withBody', { get() { return api.body(); } });
    Object.defineProperty(api, 'json', {
      get() {
        let error = null;
        try {
          JSON.parse(response.text());
        } catch (e) {
          error = e;
        }
        fail(error === null, 'expected response body to be a valid json but got error ' + (error && error.message), 'expected response body to not be a valid json');
        return api;
      },
    });
    api.jsonBody = function (path, value) {
      const body = safeJson();
      if (path === undefined) {
        fail(body !== undefined, 'expected response body to be a valid json', 'expected response body to not be a valid json');
      } else if (arguments.length < 2) {
        fail(getPath(body, path).exists, 'expected response body json to have path ' + display(path), 'expected response body json to not have path ' + display(path));
      } else {
        const found = getPath(body, path);
        fail(found.exists && deepEqual(found.value, value), 'expected response body json at ' + display(path) + ' to contain ' + display(value) + ' but got ' + display(found.value), 'expected response body json at ' + display(path) + ' to not contain ' + display(value));
      }
      return api;
    };
    api.responseTime = {
      below: (ms) => {
        fail(response.responseTime < ms, 'expected response time to be below ' + ms + ' but got ' + response.responseTime, 'expected response time to not be below ' + ms);
        return api;
      },
    };
    return api;
  }

  // ---------- pm ----------

  const pm = {};
  pm.environment = environment.api;
  pm.globals = globals.api;
  pm.collectionVariables = collection.api;
  pm.variables = {
    get: (key) => lookup(String(key)),
    set: (key, value) => {
      key = String(key);
      local.set(key, value);
      record('local', 'set', key, asText(value));
    },
    unset: (key) => {
      key = String(key);
      if (local.delete(key)) record('local', 'unset', key);
    },
    has: (key) => lookup(String(key)) !== undefined,
    toObject: () => {
      const object = {};
      for (let i = layers.length - 1; i >= 0; i -= 1) layers[i].forEach((value, key) => (object[key] = value));
      return object;
    },
    replaceIn: (text) => replaceIn(text, lookup),
  };
  pm.iterationData = {
    get: (key) => (iteration.has(String(key)) ? iteration.get(String(key)) : undefined),
    has: (key) => iteration.has(String(key)),
    toObject: () => Object.fromEntries(iteration),
    toJSON: () => Object.fromEntries(iteration),
  };
  pm.info = Object.freeze({
    eventName: phase,
    requestName: input.info.requestName,
    requestId: input.info.requestId,
    iteration: input.info.iteration,
    iterationCount: input.info.iterationCount,
  });
  pm.execution = Object.freeze({
    skipRequest: () => {
      if (phase === 'prerequest') out.skip = true;
    },
  });
  const request = input.request || { url: '', method: 'GET', headers: [] };
  pm.request = {
    url: makeUrl(request.url),
    method: request.method,
    headers: headerList(request.headers, phase === 'prerequest'),
    body: makeBody(request.body),
  };
  pm.request.toJSON = () => ({ url: String(request.url), method: request.method, header: pm.request.headers.all() });
  if (input.response) pm.response = makeResponse(input.response);

  pm.test = function (name, fn) {
    const entry = { name: String(name), passed: true, msg: null };
    out.tests.push(entry);
    const failWith = (error) => {
      entry.passed = false;
      entry.msg = describe(error);
    };
    if (typeof fn !== 'function') {
      entry.skipped = true;
      return pm;
    }
    try {
      const result = fn();
      if (result && typeof result.then === 'function') result.then(undefined, failWith);
    } catch (error) {
      failWith(error);
    }
    return pm;
  };
  pm.test.skip = function (name) {
    out.tests.push({ name: String(name), passed: true, msg: null, skipped: true });
    return pm;
  };
  pm.expect = (value, message) => new Assertion(value, message);
  pm.expect.fail = (message) => {
    throw new AssertionError(message || 'expect.fail()');
  };

  function sendOptions(request) {
    if (typeof request === 'string') return { url: request, method: 'GET', header: [], body: null };
    if (!request || request.url === undefined) throw new TypeError('pm.sendRequest needs a URL or {url, method, header, body}');
    let header = request.header || request.headers || [];
    if (!Array.isArray(header)) header = Object.keys(header).map((key) => ({ key: key, value: header[key] }));
    let body = null;
    if (request.body) {
      const mode = request.body.mode || 'raw';
      if (mode === 'raw') body = { mode: 'raw', raw: asText(request.body.raw) };
      else if (mode === 'urlencoded') body = { mode: 'urlencoded', urlencoded: (request.body.urlencoded || []).map((p) => ({ key: String(p.key), value: asText(p.value) })) };
      else throw new TypeError('pm.sendRequest: body mode ' + inspect(mode) + ' is not supported (use raw or urlencoded)');
    }
    return {
      url: String(request.url),
      method: String(request.method || 'GET').toUpperCase(),
      header: header.filter((h) => !h.disabled).map((h) => ({ key: String(h.key), value: asText(h.value) })),
      body: body,
    };
  }

  pm.sendRequest = function (request, callback) {
    let error = null;
    let response = null;
    if (++sends > MAX_SENDS) {
      error = new Error('pm.sendRequest: no more than ' + MAX_SENDS + ' calls per script');
    } else {
      try {
        const result = JSON.parse(native.send(JSON.stringify(sendOptions(request))));
        if (result.error) error = new Error(result.error);
        else response = makeResponse(result.response);
      } catch (e) {
        error = e;
      }
    }
    if (typeof callback === 'function') {
      addTimer(callback, 0, [error, response], false);
      return undefined;
    }
    return error ? Promise.reject(error) : Promise.resolve(response);
  };

  pm.cookies = unsupported('pm.cookies', 'available in a later version');
  pm.visualizer = unsupported('pm.visualizer');
  g.pm = Object.freeze(pm);

  g.__rp_finish = function () {
    return JSON.stringify(out);
  };
})();
