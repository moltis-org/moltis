var __defProp = Object.defineProperty;
var __defNormalProp = (obj, key, value) => key in obj ? __defProp(obj, key, { enumerable: true, configurable: true, writable: true, value }) : obj[key] = value;
var __publicField = (obj, key, value) => __defNormalProp(obj, typeof key !== "symbol" ? key + "" : key, value);
var _a;
var n$1, l$3, u$2, t$3, i$2, r$2, o$2, e$2, f$1, c$2, s$2, a$2, h$3, p$3, v$3, d$3 = {}, w$4 = [], _$3 = /acit|ex(?:s|g|n|p|$)|rph|grid|ows|mnc|ntw|ine[ch]|zoo|^ord|itera/i, g$3 = Array.isArray;
function m$3(n2, l2) {
  for (var u4 in l2) n2[u4] = l2[u4];
  return n2;
}
function b$3(n2) {
  n2 && n2.parentNode && n2.parentNode.removeChild(n2);
}
function k$2(l2, u4, t2) {
  var i2, r2, o2, e2 = {};
  for (o2 in u4) "key" == o2 ? i2 = u4[o2] : "ref" == o2 ? r2 = u4[o2] : e2[o2] = u4[o2];
  if (arguments.length > 2 && (e2.children = arguments.length > 3 ? n$1.call(arguments, 2) : t2), "function" == typeof l2 && null != l2.defaultProps) for (o2 in l2.defaultProps) void 0 === e2[o2] && (e2[o2] = l2.defaultProps[o2]);
  return x$3(l2, e2, i2, r2, null);
}
function x$3(n2, t2, i2, r2, o2) {
  var e2 = { type: n2, props: t2, key: i2, ref: r2, __k: null, __: null, __b: 0, __e: null, __c: null, constructor: void 0, __v: null == o2 ? ++u$2 : o2, __i: -1, __u: 0 };
  return null == o2 && null != l$3.vnode && l$3.vnode(e2), e2;
}
function S$2(n2) {
  return n2.children;
}
function C$2(n2, l2) {
  this.props = n2, this.context = l2;
}
function $$2(n2, l2) {
  if (null == l2) return n2.__ ? $$2(n2.__, n2.__i + 1) : null;
  for (var u4; l2 < n2.__k.length; l2++) if (null != (u4 = n2.__k[l2]) && null != u4.__e) return u4.__e;
  return "function" == typeof n2.type ? $$2(n2) : null;
}
function I$1(n2) {
  if (n2.__P && n2.__d) {
    var u4 = n2.__v, t2 = u4.__e, i2 = [], r2 = [], o2 = m$3({}, u4);
    o2.__v = u4.__v + 1, l$3.vnode && l$3.vnode(o2), q$3(n2.__P, o2, u4, n2.__n, n2.__P.namespaceURI, 32 & u4.__u ? [t2] : null, i2, null == t2 ? $$2(u4) : t2, !!(32 & u4.__u), r2), o2.__v = u4.__v, o2.__.__k[o2.__i] = o2, D$2(i2, o2, r2), u4.__e = u4.__ = null, o2.__e != t2 && P$1(o2);
  }
}
function P$1(n2) {
  if (null != (n2 = n2.__) && null != n2.__c) return n2.__e = n2.__c.base = null, n2.__k.some(function(l2) {
    if (null != l2 && null != l2.__e) return n2.__e = n2.__c.base = l2.__e;
  }), P$1(n2);
}
function A$1(n2) {
  (!n2.__d && (n2.__d = true) && i$2.push(n2) && !H$1.__r++ || r$2 != l$3.debounceRendering) && ((r$2 = l$3.debounceRendering) || o$2)(H$1);
}
function H$1() {
  try {
    for (var n2, l2 = 1; i$2.length; ) i$2.length > l2 && i$2.sort(e$2), n2 = i$2.shift(), l2 = i$2.length, I$1(n2);
  } finally {
    i$2.length = H$1.__r = 0;
  }
}
function L$1(n2, l2, u4, t2, i2, r2, o2, e2, f2, c2, s2) {
  var a2, h2, p2, v2, y3, _2, g2, m2 = t2 && t2.__k || w$4, b2 = l2.length;
  for (f2 = T$2(u4, l2, m2, f2, b2), a2 = 0; a2 < b2; a2++) null != (p2 = u4.__k[a2]) && (h2 = -1 != p2.__i && m2[p2.__i] || d$3, p2.__i = a2, _2 = q$3(n2, p2, h2, i2, r2, o2, e2, f2, c2, s2), v2 = p2.__e, p2.ref && h2.ref != p2.ref && (h2.ref && J$1(h2.ref, null, p2), s2.push(p2.ref, p2.__c || v2, p2)), null == y3 && null != v2 && (y3 = v2), (g2 = !!(4 & p2.__u)) || h2.__k === p2.__k ? (f2 = j$3(p2, f2, n2, g2), g2 && h2.__e && (h2.__e = null)) : "function" == typeof p2.type && void 0 !== _2 ? f2 = _2 : v2 && (f2 = v2.nextSibling), p2.__u &= -7);
  return u4.__e = y3, f2;
}
function T$2(n2, l2, u4, t2, i2) {
  var r2, o2, e2, f2, c2, s2 = u4.length, a2 = s2, h2 = 0;
  for (n2.__k = new Array(i2), r2 = 0; r2 < i2; r2++) null != (o2 = l2[r2]) && "boolean" != typeof o2 && "function" != typeof o2 ? ("string" == typeof o2 || "number" == typeof o2 || "bigint" == typeof o2 || o2.constructor == String ? o2 = n2.__k[r2] = x$3(null, o2, null, null, null) : g$3(o2) ? o2 = n2.__k[r2] = x$3(S$2, { children: o2 }, null, null, null) : void 0 === o2.constructor && o2.__b > 0 ? o2 = n2.__k[r2] = x$3(o2.type, o2.props, o2.key, o2.ref ? o2.ref : null, o2.__v) : n2.__k[r2] = o2, f2 = r2 + h2, o2.__ = n2, o2.__b = n2.__b + 1, e2 = null, -1 != (c2 = o2.__i = O$1(o2, u4, f2, a2)) && (a2--, (e2 = u4[c2]) && (e2.__u |= 2)), null == e2 || null == e2.__v ? (-1 == c2 && (i2 > s2 ? h2-- : i2 < s2 && h2++), "function" != typeof o2.type && (o2.__u |= 4)) : c2 != f2 && (c2 == f2 - 1 ? h2-- : c2 == f2 + 1 ? h2++ : (c2 > f2 ? h2-- : h2++, o2.__u |= 4))) : n2.__k[r2] = null;
  if (a2) for (r2 = 0; r2 < s2; r2++) null != (e2 = u4[r2]) && 0 == (2 & e2.__u) && (e2.__e == t2 && (t2 = $$2(e2)), K$1(e2, e2));
  return t2;
}
function j$3(n2, l2, u4, t2) {
  var i2, r2;
  if ("function" == typeof n2.type) {
    for (i2 = n2.__k, r2 = 0; i2 && r2 < i2.length; r2++) i2[r2] && (i2[r2].__ = n2, l2 = j$3(i2[r2], l2, u4, t2));
    return l2;
  }
  n2.__e != l2 && (t2 && (l2 && n2.type && !l2.parentNode && (l2 = $$2(n2)), u4.insertBefore(n2.__e, l2 || null)), l2 = n2.__e);
  do {
    l2 = l2 && l2.nextSibling;
  } while (null != l2 && 8 == l2.nodeType);
  return l2;
}
function O$1(n2, l2, u4, t2) {
  var i2, r2, o2, e2 = n2.key, f2 = n2.type, c2 = l2[u4], s2 = null != c2 && 0 == (2 & c2.__u);
  if (null === c2 && null == e2 || s2 && e2 == c2.key && f2 == c2.type) return u4;
  if (t2 > (s2 ? 1 : 0)) {
    for (i2 = u4 - 1, r2 = u4 + 1; i2 >= 0 || r2 < l2.length; ) if (null != (c2 = l2[o2 = i2 >= 0 ? i2-- : r2++]) && 0 == (2 & c2.__u) && e2 == c2.key && f2 == c2.type) return o2;
  }
  return -1;
}
function z$2(n2, l2, u4) {
  "-" == l2[0] ? n2.setProperty(l2, null == u4 ? "" : u4) : n2[l2] = null == u4 ? "" : "number" != typeof u4 || _$3.test(l2) ? u4 : u4 + "px";
}
function N$1(n2, l2, u4, t2, i2) {
  var r2, o2;
  n: if ("style" == l2) if ("string" == typeof u4) n2.style.cssText = u4;
  else {
    if ("string" == typeof t2 && (n2.style.cssText = t2 = ""), t2) for (l2 in t2) u4 && l2 in u4 || z$2(n2.style, l2, "");
    if (u4) for (l2 in u4) t2 && u4[l2] == t2[l2] || z$2(n2.style, l2, u4[l2]);
  }
  else if ("o" == l2[0] && "n" == l2[1]) r2 = l2 != (l2 = l2.replace(a$2, "$1")), o2 = l2.toLowerCase(), l2 = o2 in n2 || "onFocusOut" == l2 || "onFocusIn" == l2 ? o2.slice(2) : l2.slice(2), n2.l || (n2.l = {}), n2.l[l2 + r2] = u4, u4 ? t2 ? u4[s$2] = t2[s$2] : (u4[s$2] = h$3, n2.addEventListener(l2, r2 ? v$3 : p$3, r2)) : n2.removeEventListener(l2, r2 ? v$3 : p$3, r2);
  else {
    if ("http://www.w3.org/2000/svg" == i2) l2 = l2.replace(/xlink(H|:h)/, "h").replace(/sName$/, "s");
    else if ("width" != l2 && "height" != l2 && "href" != l2 && "list" != l2 && "form" != l2 && "tabIndex" != l2 && "download" != l2 && "rowSpan" != l2 && "colSpan" != l2 && "role" != l2 && "popover" != l2 && l2 in n2) try {
      n2[l2] = null == u4 ? "" : u4;
      break n;
    } catch (n3) {
    }
    "function" == typeof u4 || (null == u4 || false === u4 && "-" != l2[4] ? n2.removeAttribute(l2) : n2.setAttribute(l2, "popover" == l2 && 1 == u4 ? "" : u4));
  }
}
function V$1(n2) {
  return function(u4) {
    if (this.l) {
      var t2 = this.l[u4.type + n2];
      if (null == u4[c$2]) u4[c$2] = h$3++;
      else if (u4[c$2] < t2[s$2]) return;
      return t2(l$3.event ? l$3.event(u4) : u4);
    }
  };
}
function q$3(n2, u4, t2, i2, r2, o2, e2, f2, c2, s2) {
  var a2, h2, p2, v2, y3, d2, _2, k2, x2, M2, $2, I2, P2, A2, H2, T2 = u4.type;
  if (void 0 !== u4.constructor) return null;
  128 & t2.__u && (c2 = !!(32 & t2.__u), o2 = [f2 = u4.__e = t2.__e]), (a2 = l$3.__b) && a2(u4);
  n: if ("function" == typeof T2) try {
    if (k2 = u4.props, x2 = T2.prototype && T2.prototype.render, M2 = (a2 = T2.contextType) && i2[a2.__c], $2 = a2 ? M2 ? M2.props.value : a2.__ : i2, t2.__c ? _2 = (h2 = u4.__c = t2.__c).__ = h2.__E : (x2 ? u4.__c = h2 = new T2(k2, $2) : (u4.__c = h2 = new C$2(k2, $2), h2.constructor = T2, h2.render = Q$1), M2 && M2.sub(h2), h2.state || (h2.state = {}), h2.__n = i2, p2 = h2.__d = true, h2.__h = [], h2._sb = []), x2 && null == h2.__s && (h2.__s = h2.state), x2 && null != T2.getDerivedStateFromProps && (h2.__s == h2.state && (h2.__s = m$3({}, h2.__s)), m$3(h2.__s, T2.getDerivedStateFromProps(k2, h2.__s))), v2 = h2.props, y3 = h2.state, h2.__v = u4, p2) x2 && null == T2.getDerivedStateFromProps && null != h2.componentWillMount && h2.componentWillMount(), x2 && null != h2.componentDidMount && h2.__h.push(h2.componentDidMount);
    else {
      if (x2 && null == T2.getDerivedStateFromProps && k2 !== v2 && null != h2.componentWillReceiveProps && h2.componentWillReceiveProps(k2, $2), u4.__v == t2.__v || !h2.__e && null != h2.shouldComponentUpdate && false === h2.shouldComponentUpdate(k2, h2.__s, $2)) {
        u4.__v != t2.__v && (h2.props = k2, h2.state = h2.__s, h2.__d = false), u4.__e = t2.__e, u4.__k = t2.__k, u4.__k.some(function(n3) {
          n3 && (n3.__ = u4);
        }), w$4.push.apply(h2.__h, h2._sb), h2._sb = [], h2.__h.length && e2.push(h2);
        break n;
      }
      null != h2.componentWillUpdate && h2.componentWillUpdate(k2, h2.__s, $2), x2 && null != h2.componentDidUpdate && h2.__h.push(function() {
        h2.componentDidUpdate(v2, y3, d2);
      });
    }
    if (h2.context = $2, h2.props = k2, h2.__P = n2, h2.__e = false, I2 = l$3.__r, P2 = 0, x2) h2.state = h2.__s, h2.__d = false, I2 && I2(u4), a2 = h2.render(h2.props, h2.state, h2.context), w$4.push.apply(h2.__h, h2._sb), h2._sb = [];
    else do {
      h2.__d = false, I2 && I2(u4), a2 = h2.render(h2.props, h2.state, h2.context), h2.state = h2.__s;
    } while (h2.__d && ++P2 < 25);
    h2.state = h2.__s, null != h2.getChildContext && (i2 = m$3(m$3({}, i2), h2.getChildContext())), x2 && !p2 && null != h2.getSnapshotBeforeUpdate && (d2 = h2.getSnapshotBeforeUpdate(v2, y3)), A2 = null != a2 && a2.type === S$2 && null == a2.key ? E$2(a2.props.children) : a2, f2 = L$1(n2, g$3(A2) ? A2 : [A2], u4, t2, i2, r2, o2, e2, f2, c2, s2), h2.base = u4.__e, u4.__u &= -161, h2.__h.length && e2.push(h2), _2 && (h2.__E = h2.__ = null);
  } catch (n3) {
    if (u4.__v = null, c2 || null != o2) if (n3.then) {
      for (u4.__u |= c2 ? 160 : 128; f2 && 8 == f2.nodeType && f2.nextSibling; ) f2 = f2.nextSibling;
      o2[o2.indexOf(f2)] = null, u4.__e = f2;
    } else {
      for (H2 = o2.length; H2--; ) b$3(o2[H2]);
      B$2(u4);
    }
    else u4.__e = t2.__e, u4.__k = t2.__k, n3.then || B$2(u4);
    l$3.__e(n3, u4, t2);
  }
  else null == o2 && u4.__v == t2.__v ? (u4.__k = t2.__k, u4.__e = t2.__e) : f2 = u4.__e = G$1(t2.__e, u4, t2, i2, r2, o2, e2, c2, s2);
  return (a2 = l$3.diffed) && a2(u4), 128 & u4.__u ? void 0 : f2;
}
function B$2(n2) {
  n2 && (n2.__c && (n2.__c.__e = true), n2.__k && n2.__k.some(B$2));
}
function D$2(n2, u4, t2) {
  for (var i2 = 0; i2 < t2.length; i2++) J$1(t2[i2], t2[++i2], t2[++i2]);
  l$3.__c && l$3.__c(u4, n2), n2.some(function(u5) {
    try {
      n2 = u5.__h, u5.__h = [], n2.some(function(n3) {
        n3.call(u5);
      });
    } catch (n3) {
      l$3.__e(n3, u5.__v);
    }
  });
}
function E$2(n2) {
  return "object" != typeof n2 || null == n2 || n2.__b > 0 ? n2 : g$3(n2) ? n2.map(E$2) : m$3({}, n2);
}
function G$1(u4, t2, i2, r2, o2, e2, f2, c2, s2) {
  var a2, h2, p2, v2, y3, w3, _2, m2 = i2.props || d$3, k2 = t2.props, x2 = t2.type;
  if ("svg" == x2 ? o2 = "http://www.w3.org/2000/svg" : "math" == x2 ? o2 = "http://www.w3.org/1998/Math/MathML" : o2 || (o2 = "http://www.w3.org/1999/xhtml"), null != e2) {
    for (a2 = 0; a2 < e2.length; a2++) if ((y3 = e2[a2]) && "setAttribute" in y3 == !!x2 && (x2 ? y3.localName == x2 : 3 == y3.nodeType)) {
      u4 = y3, e2[a2] = null;
      break;
    }
  }
  if (null == u4) {
    if (null == x2) return document.createTextNode(k2);
    u4 = document.createElementNS(o2, x2, k2.is && k2), c2 && (l$3.__m && l$3.__m(t2, e2), c2 = false), e2 = null;
  }
  if (null == x2) m2 === k2 || c2 && u4.data == k2 || (u4.data = k2);
  else {
    if (e2 = e2 && n$1.call(u4.childNodes), !c2 && null != e2) for (m2 = {}, a2 = 0; a2 < u4.attributes.length; a2++) m2[(y3 = u4.attributes[a2]).name] = y3.value;
    for (a2 in m2) y3 = m2[a2], "dangerouslySetInnerHTML" == a2 ? p2 = y3 : "children" == a2 || a2 in k2 || "value" == a2 && "defaultValue" in k2 || "checked" == a2 && "defaultChecked" in k2 || N$1(u4, a2, null, y3, o2);
    for (a2 in k2) y3 = k2[a2], "children" == a2 ? v2 = y3 : "dangerouslySetInnerHTML" == a2 ? h2 = y3 : "value" == a2 ? w3 = y3 : "checked" == a2 ? _2 = y3 : c2 && "function" != typeof y3 || m2[a2] === y3 || N$1(u4, a2, y3, m2[a2], o2);
    if (h2) c2 || p2 && (h2.__html == p2.__html || h2.__html == u4.innerHTML) || (u4.innerHTML = h2.__html), t2.__k = [];
    else if (p2 && (u4.innerHTML = ""), L$1("template" == t2.type ? u4.content : u4, g$3(v2) ? v2 : [v2], t2, i2, r2, "foreignObject" == x2 ? "http://www.w3.org/1999/xhtml" : o2, e2, f2, e2 ? e2[0] : i2.__k && $$2(i2, 0), c2, s2), null != e2) for (a2 = e2.length; a2--; ) b$3(e2[a2]);
    c2 || (a2 = "value", "progress" == x2 && null == w3 ? u4.removeAttribute("value") : null != w3 && (w3 !== u4[a2] || "progress" == x2 && !w3 || "option" == x2 && w3 != m2[a2]) && N$1(u4, a2, w3, m2[a2], o2), a2 = "checked", null != _2 && _2 != u4[a2] && N$1(u4, a2, _2, m2[a2], o2));
  }
  return u4;
}
function J$1(n2, u4, t2) {
  try {
    if ("function" == typeof n2) {
      var i2 = "function" == typeof n2.__u;
      i2 && n2.__u(), i2 && null == u4 || (n2.__u = n2(u4));
    } else n2.current = u4;
  } catch (n3) {
    l$3.__e(n3, t2);
  }
}
function K$1(n2, u4, t2) {
  var i2, r2;
  if (l$3.unmount && l$3.unmount(n2), (i2 = n2.ref) && (i2.current && i2.current != n2.__e || J$1(i2, null, u4)), null != (i2 = n2.__c)) {
    if (i2.componentWillUnmount) try {
      i2.componentWillUnmount();
    } catch (n3) {
      l$3.__e(n3, u4);
    }
    i2.base = i2.__P = null;
  }
  if (i2 = n2.__k) for (r2 = 0; r2 < i2.length; r2++) i2[r2] && K$1(i2[r2], u4, t2 || "function" != typeof n2.type);
  t2 || b$3(n2.__e), n2.__c = n2.__ = n2.__e = void 0;
}
function Q$1(n2, l2, u4) {
  return this.constructor(n2, u4);
}
function R(u4, t2, i2) {
  var r2, o2, e2, f2;
  t2 == document && (t2 = document.documentElement), l$3.__ && l$3.__(u4, t2), o2 = (r2 = false) ? null : t2.__k, e2 = [], f2 = [], q$3(t2, u4 = t2.__k = k$2(S$2, null, [u4]), o2 || d$3, d$3, t2.namespaceURI, o2 ? null : t2.firstChild ? n$1.call(t2.childNodes) : null, e2, o2 ? o2.__e : t2.firstChild, r2, f2), D$2(e2, u4, f2);
}
n$1 = w$4.slice, l$3 = { __e: function(n2, l2, u4, t2) {
  for (var i2, r2, o2; l2 = l2.__; ) if ((i2 = l2.__c) && !i2.__) try {
    if ((r2 = i2.constructor) && null != r2.getDerivedStateFromError && (i2.setState(r2.getDerivedStateFromError(n2)), o2 = i2.__d), null != i2.componentDidCatch && (i2.componentDidCatch(n2, t2 || {}), o2 = i2.__d), o2) return i2.__E = i2;
  } catch (l3) {
    n2 = l3;
  }
  throw n2;
} }, u$2 = 0, t$3 = function(n2) {
  return null != n2 && void 0 === n2.constructor;
}, C$2.prototype.setState = function(n2, l2) {
  var u4;
  u4 = null != this.__s && this.__s != this.state ? this.__s : this.__s = m$3({}, this.state), "function" == typeof n2 && (n2 = n2(m$3({}, u4), this.props)), n2 && m$3(u4, n2), null != n2 && this.__v && (l2 && this._sb.push(l2), A$1(this));
}, C$2.prototype.forceUpdate = function(n2) {
  this.__v && (this.__e = true, n2 && this.__h.push(n2), A$1(this));
}, C$2.prototype.render = S$2, i$2 = [], o$2 = "function" == typeof Promise ? Promise.prototype.then.bind(Promise.resolve()) : setTimeout, e$2 = function(n2, l2) {
  return n2.__v.__b - l2.__v.__b;
}, H$1.__r = 0, f$1 = Math.random().toString(8), c$2 = "__d" + f$1, s$2 = "__a" + f$1, a$2 = /(PointerCapture)$|Capture$/i, h$3 = 0, p$3 = V$1(false), v$3 = V$1(true);
function z$1() {
  return { async: false, breaks: false, extensions: null, gfm: true, hooks: null, pedantic: false, renderer: null, silent: false, tokenizer: null, walkTokens: null };
}
var T$1 = z$1();
function G(u4) {
  T$1 = u4;
}
var _$2 = { exec: () => null };
function k$1(u4, e2 = "") {
  let t2 = typeof u4 == "string" ? u4 : u4.source, n2 = { replace: (r2, i2) => {
    let s2 = typeof i2 == "string" ? i2 : i2.source;
    return s2 = s2.replace(m$2.caret, "$1"), t2 = t2.replace(r2, s2), n2;
  }, getRegex: () => new RegExp(t2, e2) };
  return n2;
}
var Re = (() => {
  try {
    return !!new RegExp("(?<=1)(?<!1)");
  } catch {
    return false;
  }
})(), m$2 = { codeRemoveIndent: /^(?: {1,4}| {0,3}\t)/gm, outputLinkReplace: /\\([\[\]])/g, indentCodeCompensation: /^(\s+)(?:```)/, beginningSpace: /^\s+/, endingHash: /#$/, startingSpaceChar: /^ /, endingSpaceChar: / $/, nonSpaceChar: /[^ ]/, newLineCharGlobal: /\n/g, tabCharGlobal: /\t/g, multipleSpaceGlobal: /\s+/g, blankLine: /^[ \t]*$/, doubleBlankLine: /\n[ \t]*\n[ \t]*$/, blockquoteStart: /^ {0,3}>/, blockquoteSetextReplace: /\n {0,3}((?:=+|-+) *)(?=\n|$)/g, blockquoteSetextReplace2: /^ {0,3}>[ \t]?/gm, listReplaceNesting: /^ {1,4}(?=( {4})*[^ ])/g, listIsTask: /^\[[ xX]\] +\S/, listReplaceTask: /^\[[ xX]\] +/, listTaskCheckbox: /\[[ xX]\]/, anyLine: /\n.*\n/, hrefBrackets: /^<(.*)>$/, tableDelimiter: /[:|]/, tableAlignChars: /^\||\| *$/g, tableRowBlankLine: /\n[ \t]*$/, tableAlignRight: /^ *-+: *$/, tableAlignCenter: /^ *:-+: *$/, tableAlignLeft: /^ *:-+ *$/, startATag: /^<a /i, endATag: /^<\/a>/i, startPreScriptTag: /^<(pre|code|kbd|script)(\s|>)/i, endPreScriptTag: /^<\/(pre|code|kbd|script)(\s|>)/i, startAngleBracket: /^</, endAngleBracket: />$/, pedanticHrefTitle: /^([^'"]*[^\s])\s+(['"])(.*)\2/, unicodeAlphaNumeric: /[\p{L}\p{N}]/u, escapeTest: /[&<>"']/, escapeReplace: /[&<>"']/g, escapeTestNoEncode: /[<>"']|&(?!(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);)/, escapeReplaceNoEncode: /[<>"']|&(?!(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);)/g, caret: /(^|[^\[])\^/g, percentDecode: /%25/g, findPipe: /\|/g, splitPipe: / \|/, slashPipe: /\\\|/g, carriageReturn: /\r\n|\r/g, spaceLine: /^ +$/gm, notSpaceStart: /^\S*/, endingNewline: /\n$/, listItemRegex: (u4) => new RegExp(`^( {0,3}${u4})((?:[	 ][^\\n]*)?(?:\\n|$))`), nextBulletRegex: (u4) => new RegExp(`^ {0,${Math.min(3, u4 - 1)}}(?:[*+-]|\\d{1,9}[.)])((?:[ 	][^\\n]*)?(?:\\n|$))`), hrRegex: (u4) => new RegExp(`^ {0,${Math.min(3, u4 - 1)}}((?:- *){3,}|(?:_ *){3,}|(?:\\* *){3,})(?:\\n+|$)`), fencesBeginRegex: (u4) => new RegExp(`^ {0,${Math.min(3, u4 - 1)}}(?:\`\`\`|~~~)`), headingBeginRegex: (u4) => new RegExp(`^ {0,${Math.min(3, u4 - 1)}}#`), htmlBeginRegex: (u4) => new RegExp(`^ {0,${Math.min(3, u4 - 1)}}<(?:[a-z].*>|!--)`, "i"), blockquoteBeginRegex: (u4) => new RegExp(`^ {0,${Math.min(3, u4 - 1)}}>`) }, Te = /^(?:[ \t]*(?:\n|$))+/, Oe = /^((?: {4}| {0,3}\t)[^\n]+(?:\n(?:[ \t]*(?:\n|$))*)?)+/, we = /^ {0,3}(`{3,}(?=[^`\n]*(?:\n|$))|~{3,})([^\n]*)(?:\n|$)(?:|([\s\S]*?)(?:\n|$))(?: {0,3}\1[~`]* *(?=\n|$)|$)/, C$1 = /^ {0,3}((?:-[\t ]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)/, ye = /^ {0,3}(#{1,6})(?=\s|$)(.*)(?:\n+|$)/, Q = / {0,3}(?:[*+-]|\d{1,9}[.)])/, ie = /^(?!bull |blockCode|fences|blockquote|heading|html|table)((?:.|\n(?!\s*?\n|bull |blockCode|fences|blockquote|heading|html|table))+?)\n {0,3}(=+|-+) *(?:\n+|$)/, oe = k$1(ie).replace(/bull/g, Q).replace(/blockCode/g, /(?: {4}| {0,3}\t)/).replace(/fences/g, / {0,3}(?:`{3,}|~{3,})/).replace(/blockquote/g, / {0,3}>/).replace(/heading/g, / {0,3}#{1,6}/).replace(/html/g, / {0,3}<[^\n>]+>\n/).replace(/\|table/g, "").getRegex(), Pe = k$1(ie).replace(/bull/g, Q).replace(/blockCode/g, /(?: {4}| {0,3}\t)/).replace(/fences/g, / {0,3}(?:`{3,}|~{3,})/).replace(/blockquote/g, / {0,3}>/).replace(/heading/g, / {0,3}#{1,6}/).replace(/html/g, / {0,3}<[^\n>]+>\n/).replace(/table/g, / {0,3}\|?(?:[:\- ]*\|)+[\:\- ]*\n/).getRegex(), j$2 = /^([^\n]+(?:\n(?!hr|heading|lheading|blockquote|fences|list|html|table| +\n)[^\n]+)*)/, Se = /^[^\n]+/, F$1 = /(?!\s*\])(?:\\[\s\S]|[^\[\]\\])+/, $e = k$1(/^ {0,3}\[(label)\]: *(?:\n[ \t]*)?([^<\s][^\s]*|<.*?>)(?:(?: +(?:\n[ \t]*)?| *\n[ \t]*)(title))? *(?:\n+|$)/).replace("label", F$1).replace("title", /(?:"(?:\\"?|[^"\\])*"|'[^'\n]*(?:\n[^'\n]+)*\n?'|\([^()]*\))/).getRegex(), Le = k$1(/^(bull)([ \t][^\n]+?)?(?:\n|$)/).replace(/bull/g, Q).getRegex(), v$2 = "address|article|aside|base|basefont|blockquote|body|caption|center|col|colgroup|dd|details|dialog|dir|div|dl|dt|fieldset|figcaption|figure|footer|form|frame|frameset|h[1-6]|head|header|hr|html|iframe|legend|li|link|main|menu|menuitem|meta|nav|noframes|ol|optgroup|option|p|param|search|section|summary|table|tbody|td|tfoot|th|thead|title|tr|track|ul", U = /<!--(?:-?>|[\s\S]*?(?:-->|$))/, _e = k$1("^ {0,3}(?:<(script|pre|style|textarea)[\\s>][\\s\\S]*?(?:</\\1>[^\\n]*\\n+|$)|comment[^\\n]*(\\n+|$)|<\\?[\\s\\S]*?(?:\\?>\\n*|$)|<![A-Z][\\s\\S]*?(?:>\\n*|$)|<!\\[CDATA\\[[\\s\\S]*?(?:\\]\\]>\\n*|$)|</?(tag)(?: +|\\n|/?>)[\\s\\S]*?(?:(?:\\n[ 	]*)+\\n|$)|<(?!script|pre|style|textarea)([a-z][\\w-]*)(?:attribute)*? */?>(?=[ \\t]*(?:\\n|$))[\\s\\S]*?(?:(?:\\n[ 	]*)+\\n|$)|</(?!script|pre|style|textarea)[a-z][\\w-]*\\s*>(?=[ \\t]*(?:\\n|$))[\\s\\S]*?(?:(?:\\n[ 	]*)+\\n|$))", "i").replace("comment", U).replace("tag", v$2).replace("attribute", / +[a-zA-Z:_][\w.:-]*(?: *= *"[^"\n]*"| *= *'[^'\n]*'| *= *[^\s"'=<>`]+)?/).getRegex(), ae = k$1(j$2).replace("hr", C$1).replace("heading", " {0,3}#{1,6}(?:\\s|$)").replace("|lheading", "").replace("|table", "").replace("blockquote", " {0,3}>").replace("fences", " {0,3}(?:`{3,}(?=[^`\\n]*\\n)|~{3,})[^\\n]*\\n").replace("list", " {0,3}(?:[*+-]|1[.)])[ \\t]").replace("html", "</?(?:tag)(?: +|\\n|/?>)|<(?:script|pre|style|textarea|!--)").replace("tag", v$2).getRegex(), Me = k$1(/^( {0,3}> ?(paragraph|[^\n]*)(?:\n|$))+/).replace("paragraph", ae).getRegex(), K = { blockquote: Me, code: Oe, def: $e, fences: we, heading: ye, hr: C$1, html: _e, lheading: oe, list: Le, newline: Te, paragraph: ae, table: _$2, text: Se }, re = k$1("^ *([^\\n ].*)\\n {0,3}((?:\\| *)?:?-+:? *(?:\\| *:?-+:? *)*(?:\\| *)?)(?:\\n((?:(?! *\\n|hr|heading|blockquote|code|fences|list|html).*(?:\\n|$))*)\\n*|$)").replace("hr", C$1).replace("heading", " {0,3}#{1,6}(?:\\s|$)").replace("blockquote", " {0,3}>").replace("code", "(?: {4}| {0,3}	)[^\\n]").replace("fences", " {0,3}(?:`{3,}(?=[^`\\n]*\\n)|~{3,})[^\\n]*\\n").replace("list", " {0,3}(?:[*+-]|1[.)])[ \\t]").replace("html", "</?(?:tag)(?: +|\\n|/?>)|<(?:script|pre|style|textarea|!--)").replace("tag", v$2).getRegex(), ze = { ...K, lheading: Pe, table: re, paragraph: k$1(j$2).replace("hr", C$1).replace("heading", " {0,3}#{1,6}(?:\\s|$)").replace("|lheading", "").replace("table", re).replace("blockquote", " {0,3}>").replace("fences", " {0,3}(?:`{3,}(?=[^`\\n]*\\n)|~{3,})[^\\n]*\\n").replace("list", " {0,3}(?:[*+-]|1[.)])[ \\t]").replace("html", "</?(?:tag)(?: +|\\n|/?>)|<(?:script|pre|style|textarea|!--)").replace("tag", v$2).getRegex() }, Ee = { ...K, html: k$1(`^ *(?:comment *(?:\\n|\\s*$)|<(tag)[\\s\\S]+?</\\1> *(?:\\n{2,}|\\s*$)|<tag(?:"[^"]*"|'[^']*'|\\s[^'"/>\\s]*)*?/?> *(?:\\n{2,}|\\s*$))`).replace("comment", U).replace(/tag/g, "(?!(?:a|em|strong|small|s|cite|q|dfn|abbr|data|time|code|var|samp|kbd|sub|sup|i|b|u|mark|ruby|rt|rp|bdi|bdo|span|br|wbr|ins|del|img)\\b)\\w+(?!:|[^\\w\\s@]*@)\\b").getRegex(), def: /^ *\[([^\]]+)\]: *<?([^\s>]+)>?(?: +(["(][^\n]+[")]))? *(?:\n+|$)/, heading: /^(#{1,6})(.*)(?:\n+|$)/, fences: _$2, lheading: /^(.+?)\n {0,3}(=+|-+) *(?:\n+|$)/, paragraph: k$1(j$2).replace("hr", C$1).replace("heading", ` *#{1,6} *[^
]`).replace("lheading", oe).replace("|table", "").replace("blockquote", " {0,3}>").replace("|fences", "").replace("|list", "").replace("|html", "").replace("|tag", "").getRegex() }, Ie = /^\\([!"#$%&'()*+,\-./:;<=>?@\[\]\\^_`{|}~])/, Ae = /^(`+)([^`]|[^`][\s\S]*?[^`])\1(?!`)/, le = /^( {2,}|\\)\n(?!\s*$)/, Ce = /^(`+|[^`])(?:(?= {2,}\n)|[\s\S]*?(?:(?=[\\<!\[`*_]|\b_|$)|[^ ](?= {2,}\n)))/, E$1 = /[\p{P}\p{S}]/u, H = /[\s\p{P}\p{S}]/u, W = /[^\s\p{P}\p{S}]/u, Be = k$1(/^((?![*_])punctSpace)/, "u").replace(/punctSpace/g, H).getRegex(), ue = /(?!~)[\p{P}\p{S}]/u, De = /(?!~)[\s\p{P}\p{S}]/u, qe = /(?:[^\s\p{P}\p{S}]|~)/u, ve = k$1(/link|precode-code|html/, "g").replace("link", /\[(?:[^\[\]`]|(?<a>`+)[^`]+\k<a>(?!`))*?\]\((?:\\[\s\S]|[^\\\(\)]|\((?:\\[\s\S]|[^\\\(\)])*\))*\)/).replace("precode-", Re ? "(?<!`)()" : "(^^|[^`])").replace("code", /(?<b>`+)[^`]+\k<b>(?!`)/).replace("html", /<(?! )[^<>]*?>/).getRegex(), pe = /^(?:\*+(?:((?!\*)punct)|([^\s*]))?)|^_+(?:((?!_)punct)|([^\s_]))?/, He = k$1(pe, "u").replace(/punct/g, E$1).getRegex(), Ze = k$1(pe, "u").replace(/punct/g, ue).getRegex(), ce = "^[^_*]*?__[^_*]*?\\*[^_*]*?(?=__)|[^*]+(?=[^*])|(?!\\*)punct(\\*+)(?=[\\s]|$)|notPunctSpace(\\*+)(?!\\*)(?=punctSpace|$)|(?!\\*)punctSpace(\\*+)(?=notPunctSpace)|[\\s](\\*+)(?!\\*)(?=punct)|(?!\\*)punct(\\*+)(?!\\*)(?=punct)|notPunctSpace(\\*+)(?=notPunctSpace)", Ge = k$1(ce, "gu").replace(/notPunctSpace/g, W).replace(/punctSpace/g, H).replace(/punct/g, E$1).getRegex(), Ne = k$1(ce, "gu").replace(/notPunctSpace/g, qe).replace(/punctSpace/g, De).replace(/punct/g, ue).getRegex(), Qe = k$1("^[^_*]*?\\*\\*[^_*]*?_[^_*]*?(?=\\*\\*)|[^_]+(?=[^_])|(?!_)punct(_+)(?=[\\s]|$)|notPunctSpace(_+)(?!_)(?=punctSpace|$)|(?!_)punctSpace(_+)(?=notPunctSpace)|[\\s](_+)(?!_)(?=punct)|(?!_)punct(_+)(?!_)(?=punct)", "gu").replace(/notPunctSpace/g, W).replace(/punctSpace/g, H).replace(/punct/g, E$1).getRegex(), je = k$1(/^~~?(?:((?!~)punct)|[^\s~])/, "u").replace(/punct/g, E$1).getRegex(), Fe = "^[^~]+(?=[^~])|(?!~)punct(~~?)(?=[\\s]|$)|notPunctSpace(~~?)(?!~)(?=punctSpace|$)|(?!~)punctSpace(~~?)(?=notPunctSpace)|[\\s](~~?)(?!~)(?=punct)|(?!~)punct(~~?)(?!~)(?=punct)|notPunctSpace(~~?)(?=notPunctSpace)", Ue = k$1(Fe, "gu").replace(/notPunctSpace/g, W).replace(/punctSpace/g, H).replace(/punct/g, E$1).getRegex(), Ke = k$1(/\\(punct)/, "gu").replace(/punct/g, E$1).getRegex(), We = k$1(/^<(scheme:[^\s\x00-\x1f<>]*|email)>/).replace("scheme", /[a-zA-Z][a-zA-Z0-9+.-]{1,31}/).replace("email", /[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+(@)[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)+(?![-_])/).getRegex(), Xe = k$1(U).replace("(?:-->|$)", "-->").getRegex(), Je = k$1("^comment|^</[a-zA-Z][\\w:-]*\\s*>|^<[a-zA-Z][\\w-]*(?:attribute)*?\\s*/?>|^<\\?[\\s\\S]*?\\?>|^<![a-zA-Z]+\\s[\\s\\S]*?>|^<!\\[CDATA\\[[\\s\\S]*?\\]\\]>").replace("comment", Xe).replace("attribute", /\s+[a-zA-Z:_][\w.:-]*(?:\s*=\s*"[^"]*"|\s*=\s*'[^']*'|\s*=\s*[^\s"'=<>`]+)?/).getRegex(), q$2 = /(?:\[(?:\\[\s\S]|[^\[\]\\])*\]|\\[\s\S]|`+(?!`)[^`]*?`+(?!`)|``+(?=\])|[^\[\]\\`])*?/, Ve = k$1(/^!?\[(label)\]\(\s*(href)(?:(?:[ \t]+(?:\n[ \t]*)?|\n[ \t]*)(title))?\s*\)/).replace("label", q$2).replace("href", /<(?:\\.|[^\n<>\\])+>|[^ \t\n\x00-\x1f]*/).replace("title", /"(?:\\"?|[^"\\])*"|'(?:\\'?|[^'\\])*'|\((?:\\\)?|[^)\\])*\)/).getRegex(), he = k$1(/^!?\[(label)\]\[(ref)\]/).replace("label", q$2).replace("ref", F$1).getRegex(), ke = k$1(/^!?\[(ref)\](?:\[\])?/).replace("ref", F$1).getRegex(), Ye = k$1("reflink|nolink(?!\\()", "g").replace("reflink", he).replace("nolink", ke).getRegex(), se = /[hH][tT][tT][pP][sS]?|[fF][tT][pP]/, X = { _backpedal: _$2, anyPunctuation: Ke, autolink: We, blockSkip: ve, br: le, code: Ae, del: _$2, delLDelim: _$2, delRDelim: _$2, emStrongLDelim: He, emStrongRDelimAst: Ge, emStrongRDelimUnd: Qe, escape: Ie, link: Ve, nolink: ke, punctuation: Be, reflink: he, reflinkSearch: Ye, tag: Je, text: Ce, url: _$2 }, et = { ...X, link: k$1(/^!?\[(label)\]\((.*?)\)/).replace("label", q$2).getRegex(), reflink: k$1(/^!?\[(label)\]\s*\[([^\]]*)\]/).replace("label", q$2).getRegex() }, N = { ...X, emStrongRDelimAst: Ne, emStrongLDelim: Ze, delLDelim: je, delRDelim: Ue, url: k$1(/^((?:protocol):\/\/|www\.)(?:[a-zA-Z0-9\-]+\.?)+[^\s<]*|^email/).replace("protocol", se).replace("email", /[A-Za-z0-9._+-]+(@)[a-zA-Z0-9-_]+(?:\.[a-zA-Z0-9-_]*[a-zA-Z0-9])+(?![-_])/).getRegex(), _backpedal: /(?:[^?!.,:;*_'"~()&]+|\([^)]*\)|&(?![a-zA-Z0-9]+;$)|[?!.,:;*_'"~)]+(?!$))+/, del: /^(~~?)(?=[^\s~])((?:\\[\s\S]|[^\\])*?(?:\\[\s\S]|[^\s~\\]))\1(?=[^~]|$)/, text: k$1(/^([`~]+|[^`~])(?:(?= {2,}\n)|(?=[a-zA-Z0-9.!#$%&'*+\/=?_`{\|}~-]+@)|[\s\S]*?(?:(?=[\\<!\[`*~_]|\b_|protocol:\/\/|www\.|$)|[^ ](?= {2,}\n)|[^a-zA-Z0-9.!#$%&'*+\/=?_`{\|}~-](?=[a-zA-Z0-9.!#$%&'*+\/=?_`{\|}~-]+@)))/).replace("protocol", se).getRegex() }, tt = { ...N, br: k$1(le).replace("{2,}", "*").getRegex(), text: k$1(N.text).replace("\\b_", "\\b_| {2,}\\n").replace(/\{2,\}/g, "*").getRegex() }, B$1 = { normal: K, gfm: ze, pedantic: Ee }, I = { normal: X, gfm: N, breaks: tt, pedantic: et };
var nt = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }, de = (u4) => nt[u4];
function O(u4, e2) {
  if (e2) {
    if (m$2.escapeTest.test(u4)) return u4.replace(m$2.escapeReplace, de);
  } else if (m$2.escapeTestNoEncode.test(u4)) return u4.replace(m$2.escapeReplaceNoEncode, de);
  return u4;
}
function J(u4) {
  try {
    u4 = encodeURI(u4).replace(m$2.percentDecode, "%");
  } catch {
    return null;
  }
  return u4;
}
function V(u4, e2) {
  var _a2;
  let t2 = u4.replace(m$2.findPipe, (i2, s2, a2) => {
    let o2 = false, l2 = s2;
    for (; --l2 >= 0 && a2[l2] === "\\"; ) o2 = !o2;
    return o2 ? "|" : " |";
  }), n2 = t2.split(m$2.splitPipe), r2 = 0;
  if (n2[0].trim() || n2.shift(), n2.length > 0 && !((_a2 = n2.at(-1)) == null ? void 0 : _a2.trim()) && n2.pop(), e2) if (n2.length > e2) n2.splice(e2);
  else for (; n2.length < e2; ) n2.push("");
  for (; r2 < n2.length; r2++) n2[r2] = n2[r2].trim().replace(m$2.slashPipe, "|");
  return n2;
}
function $$1(u4, e2, t2) {
  let n2 = u4.length;
  if (n2 === 0) return "";
  let r2 = 0;
  for (; r2 < n2; ) {
    let i2 = u4.charAt(n2 - r2 - 1);
    if (i2 === e2 && true) r2++;
    else break;
  }
  return u4.slice(0, n2 - r2);
}
function Y(u4) {
  let e2 = u4.split(`
`), t2 = e2.length - 1;
  for (; t2 >= 0 && !e2[t2].trim(); ) t2--;
  return e2.length - t2 <= 2 ? u4 : e2.slice(0, t2 + 1).join(`
`);
}
function ge(u4, e2) {
  if (u4.indexOf(e2[1]) === -1) return -1;
  let t2 = 0;
  for (let n2 = 0; n2 < u4.length; n2++) if (u4[n2] === "\\") n2++;
  else if (u4[n2] === e2[0]) t2++;
  else if (u4[n2] === e2[1] && (t2--, t2 < 0)) return n2;
  return t2 > 0 ? -2 : -1;
}
function fe(u4, e2 = 0) {
  let t2 = e2, n2 = "";
  for (let r2 of u4) if (r2 === "	") {
    let i2 = 4 - t2 % 4;
    n2 += " ".repeat(i2), t2 += i2;
  } else n2 += r2, t2++;
  return n2;
}
function me(u4, e2, t2, n2, r2) {
  let i2 = e2.href, s2 = e2.title || null, a2 = u4[1].replace(r2.other.outputLinkReplace, "$1");
  n2.state.inLink = true;
  let o2 = { type: u4[0].charAt(0) === "!" ? "image" : "link", raw: t2, href: i2, title: s2, text: a2, tokens: n2.inlineTokens(a2) };
  return n2.state.inLink = false, o2;
}
function rt(u4, e2, t2) {
  let n2 = u4.match(t2.other.indentCodeCompensation);
  if (n2 === null) return e2;
  let r2 = n2[1];
  return e2.split(`
`).map((i2) => {
    let s2 = i2.match(t2.other.beginningSpace);
    if (s2 === null) return i2;
    let [a2] = s2;
    return a2.length >= r2.length ? i2.slice(r2.length) : i2;
  }).join(`
`);
}
var w$3 = class w {
  constructor(e2) {
    __publicField(this, "options");
    __publicField(this, "rules");
    __publicField(this, "lexer");
    this.options = e2 || T$1;
  }
  space(e2) {
    let t2 = this.rules.block.newline.exec(e2);
    if (t2 && t2[0].length > 0) return { type: "space", raw: t2[0] };
  }
  code(e2) {
    let t2 = this.rules.block.code.exec(e2);
    if (t2) {
      let n2 = this.options.pedantic ? t2[0] : Y(t2[0]), r2 = n2.replace(this.rules.other.codeRemoveIndent, "");
      return { type: "code", raw: n2, codeBlockStyle: "indented", text: r2 };
    }
  }
  fences(e2) {
    let t2 = this.rules.block.fences.exec(e2);
    if (t2) {
      let n2 = t2[0], r2 = rt(n2, t2[3] || "", this.rules);
      return { type: "code", raw: n2, lang: t2[2] ? t2[2].trim().replace(this.rules.inline.anyPunctuation, "$1") : t2[2], text: r2 };
    }
  }
  heading(e2) {
    let t2 = this.rules.block.heading.exec(e2);
    if (t2) {
      let n2 = t2[2].trim();
      if (this.rules.other.endingHash.test(n2)) {
        let r2 = $$1(n2, "#");
        (this.options.pedantic || !r2 || this.rules.other.endingSpaceChar.test(r2)) && (n2 = r2.trim());
      }
      return { type: "heading", raw: $$1(t2[0], `
`), depth: t2[1].length, text: n2, tokens: this.lexer.inline(n2) };
    }
  }
  hr(e2) {
    let t2 = this.rules.block.hr.exec(e2);
    if (t2) return { type: "hr", raw: $$1(t2[0], `
`) };
  }
  blockquote(e2) {
    let t2 = this.rules.block.blockquote.exec(e2);
    if (t2) {
      let n2 = $$1(t2[0], `
`).split(`
`), r2 = "", i2 = "", s2 = [];
      for (; n2.length > 0; ) {
        let a2 = false, o2 = [], l2;
        for (l2 = 0; l2 < n2.length; l2++) if (this.rules.other.blockquoteStart.test(n2[l2])) o2.push(n2[l2]), a2 = true;
        else if (!a2) o2.push(n2[l2]);
        else break;
        n2 = n2.slice(l2);
        let p2 = o2.join(`
`), c2 = p2.replace(this.rules.other.blockquoteSetextReplace, `
    $1`).replace(this.rules.other.blockquoteSetextReplace2, "");
        r2 = r2 ? `${r2}
${p2}` : p2, i2 = i2 ? `${i2}
${c2}` : c2;
        let d2 = this.lexer.state.top;
        if (this.lexer.state.top = true, this.lexer.blockTokens(c2, s2, true), this.lexer.state.top = d2, n2.length === 0) break;
        let h2 = s2.at(-1);
        if ((h2 == null ? void 0 : h2.type) === "code") break;
        if ((h2 == null ? void 0 : h2.type) === "blockquote") {
          let R2 = h2, f2 = R2.raw + `
` + n2.join(`
`), S2 = this.blockquote(f2);
          s2[s2.length - 1] = S2, r2 = r2.substring(0, r2.length - R2.raw.length) + S2.raw, i2 = i2.substring(0, i2.length - R2.text.length) + S2.text;
          break;
        } else if ((h2 == null ? void 0 : h2.type) === "list") {
          let R2 = h2, f2 = R2.raw + `
` + n2.join(`
`), S2 = this.list(f2);
          s2[s2.length - 1] = S2, r2 = r2.substring(0, r2.length - h2.raw.length) + S2.raw, i2 = i2.substring(0, i2.length - R2.raw.length) + S2.raw, n2 = f2.substring(s2.at(-1).raw.length).split(`
`);
          continue;
        }
      }
      return { type: "blockquote", raw: r2, tokens: s2, text: i2 };
    }
  }
  list(e2) {
    var _a2, _b;
    let t2 = this.rules.block.list.exec(e2);
    if (t2) {
      let n2 = t2[1].trim(), r2 = n2.length > 1, i2 = { type: "list", raw: "", ordered: r2, start: r2 ? +n2.slice(0, -1) : "", loose: false, items: [] };
      n2 = r2 ? `\\d{1,9}\\${n2.slice(-1)}` : `\\${n2}`, this.options.pedantic && (n2 = r2 ? n2 : "[*+-]");
      let s2 = this.rules.other.listItemRegex(n2), a2 = false;
      for (; e2; ) {
        let l2 = false, p2 = "", c2 = "";
        if (!(t2 = s2.exec(e2)) || this.rules.block.hr.test(e2)) break;
        p2 = t2[0], e2 = e2.substring(p2.length);
        let d2 = fe(t2[2].split(`
`, 1)[0], t2[1].length), h2 = e2.split(`
`, 1)[0], R2 = !d2.trim(), f2 = 0;
        if (this.options.pedantic ? (f2 = 2, c2 = d2.trimStart()) : R2 ? f2 = t2[1].length + 1 : (f2 = d2.search(this.rules.other.nonSpaceChar), f2 = f2 > 4 ? 1 : f2, c2 = d2.slice(f2), f2 += t2[1].length), R2 && this.rules.other.blankLine.test(h2) && (p2 += h2 + `
`, e2 = e2.substring(h2.length + 1), l2 = true), !l2) {
          let S2 = this.rules.other.nextBulletRegex(f2), ee = this.rules.other.hrRegex(f2), te = this.rules.other.fencesBeginRegex(f2), ne = this.rules.other.headingBeginRegex(f2), xe = this.rules.other.htmlBeginRegex(f2), be = this.rules.other.blockquoteBeginRegex(f2);
          for (; e2; ) {
            let Z = e2.split(`
`, 1)[0], A2;
            if (h2 = Z, this.options.pedantic ? (h2 = h2.replace(this.rules.other.listReplaceNesting, "  "), A2 = h2) : A2 = h2.replace(this.rules.other.tabCharGlobal, "    "), te.test(h2) || ne.test(h2) || xe.test(h2) || be.test(h2) || S2.test(h2) || ee.test(h2)) break;
            if (A2.search(this.rules.other.nonSpaceChar) >= f2 || !h2.trim()) c2 += `
` + A2.slice(f2);
            else {
              if (R2 || d2.replace(this.rules.other.tabCharGlobal, "    ").search(this.rules.other.nonSpaceChar) >= 4 || te.test(d2) || ne.test(d2) || ee.test(d2)) break;
              c2 += `
` + h2;
            }
            R2 = !h2.trim(), p2 += Z + `
`, e2 = e2.substring(Z.length + 1), d2 = A2.slice(f2);
          }
        }
        i2.loose || (a2 ? i2.loose = true : this.rules.other.doubleBlankLine.test(p2) && (a2 = true)), i2.items.push({ type: "list_item", raw: p2, task: !!this.options.gfm && this.rules.other.listIsTask.test(c2), loose: false, text: c2, tokens: [] }), i2.raw += p2;
      }
      let o2 = i2.items.at(-1);
      if (o2) o2.raw = o2.raw.trimEnd(), o2.text = o2.text.trimEnd();
      else return;
      i2.raw = i2.raw.trimEnd();
      for (let l2 of i2.items) {
        if (this.lexer.state.top = false, l2.tokens = this.lexer.blockTokens(l2.text, []), l2.task) {
          if (l2.text = l2.text.replace(this.rules.other.listReplaceTask, ""), ((_a2 = l2.tokens[0]) == null ? void 0 : _a2.type) === "text" || ((_b = l2.tokens[0]) == null ? void 0 : _b.type) === "paragraph") {
            l2.tokens[0].raw = l2.tokens[0].raw.replace(this.rules.other.listReplaceTask, ""), l2.tokens[0].text = l2.tokens[0].text.replace(this.rules.other.listReplaceTask, "");
            for (let c2 = this.lexer.inlineQueue.length - 1; c2 >= 0; c2--) if (this.rules.other.listIsTask.test(this.lexer.inlineQueue[c2].src)) {
              this.lexer.inlineQueue[c2].src = this.lexer.inlineQueue[c2].src.replace(this.rules.other.listReplaceTask, "");
              break;
            }
          }
          let p2 = this.rules.other.listTaskCheckbox.exec(l2.raw);
          if (p2) {
            let c2 = { type: "checkbox", raw: p2[0] + " ", checked: p2[0] !== "[ ]" };
            l2.checked = c2.checked, i2.loose ? l2.tokens[0] && ["paragraph", "text"].includes(l2.tokens[0].type) && "tokens" in l2.tokens[0] && l2.tokens[0].tokens ? (l2.tokens[0].raw = c2.raw + l2.tokens[0].raw, l2.tokens[0].text = c2.raw + l2.tokens[0].text, l2.tokens[0].tokens.unshift(c2)) : l2.tokens.unshift({ type: "paragraph", raw: c2.raw, text: c2.raw, tokens: [c2] }) : l2.tokens.unshift(c2);
          }
        }
        if (!i2.loose) {
          let p2 = l2.tokens.filter((d2) => d2.type === "space"), c2 = p2.length > 0 && p2.some((d2) => this.rules.other.anyLine.test(d2.raw));
          i2.loose = c2;
        }
      }
      if (i2.loose) for (let l2 of i2.items) {
        l2.loose = true;
        for (let p2 of l2.tokens) p2.type === "text" && (p2.type = "paragraph");
      }
      return i2;
    }
  }
  html(e2) {
    let t2 = this.rules.block.html.exec(e2);
    if (t2) {
      let n2 = Y(t2[0]);
      return { type: "html", block: true, raw: n2, pre: t2[1] === "pre" || t2[1] === "script" || t2[1] === "style", text: n2 };
    }
  }
  def(e2) {
    let t2 = this.rules.block.def.exec(e2);
    if (t2) {
      let n2 = t2[1].toLowerCase().replace(this.rules.other.multipleSpaceGlobal, " "), r2 = t2[2] ? t2[2].replace(this.rules.other.hrefBrackets, "$1").replace(this.rules.inline.anyPunctuation, "$1") : "", i2 = t2[3] ? t2[3].substring(1, t2[3].length - 1).replace(this.rules.inline.anyPunctuation, "$1") : t2[3];
      return { type: "def", tag: n2, raw: $$1(t2[0], `
`), href: r2, title: i2 };
    }
  }
  table(e2) {
    var _a2;
    let t2 = this.rules.block.table.exec(e2);
    if (!t2 || !this.rules.other.tableDelimiter.test(t2[2])) return;
    let n2 = V(t2[1]), r2 = t2[2].replace(this.rules.other.tableAlignChars, "").split("|"), i2 = ((_a2 = t2[3]) == null ? void 0 : _a2.trim()) ? t2[3].replace(this.rules.other.tableRowBlankLine, "").split(`
`) : [], s2 = { type: "table", raw: $$1(t2[0], `
`), header: [], align: [], rows: [] };
    if (n2.length === r2.length) {
      for (let a2 of r2) this.rules.other.tableAlignRight.test(a2) ? s2.align.push("right") : this.rules.other.tableAlignCenter.test(a2) ? s2.align.push("center") : this.rules.other.tableAlignLeft.test(a2) ? s2.align.push("left") : s2.align.push(null);
      for (let a2 = 0; a2 < n2.length; a2++) s2.header.push({ text: n2[a2], tokens: this.lexer.inline(n2[a2]), header: true, align: s2.align[a2] });
      for (let a2 of i2) s2.rows.push(V(a2, s2.header.length).map((o2, l2) => ({ text: o2, tokens: this.lexer.inline(o2), header: false, align: s2.align[l2] })));
      return s2;
    }
  }
  lheading(e2) {
    let t2 = this.rules.block.lheading.exec(e2);
    if (t2) {
      let n2 = t2[1].trim();
      return { type: "heading", raw: $$1(t2[0], `
`), depth: t2[2].charAt(0) === "=" ? 1 : 2, text: n2, tokens: this.lexer.inline(n2) };
    }
  }
  paragraph(e2) {
    let t2 = this.rules.block.paragraph.exec(e2);
    if (t2) {
      let n2 = t2[1].charAt(t2[1].length - 1) === `
` ? t2[1].slice(0, -1) : t2[1];
      return { type: "paragraph", raw: t2[0], text: n2, tokens: this.lexer.inline(n2) };
    }
  }
  text(e2) {
    let t2 = this.rules.block.text.exec(e2);
    if (t2) return { type: "text", raw: t2[0], text: t2[0], tokens: this.lexer.inline(t2[0]) };
  }
  escape(e2) {
    let t2 = this.rules.inline.escape.exec(e2);
    if (t2) return { type: "escape", raw: t2[0], text: t2[1] };
  }
  tag(e2) {
    let t2 = this.rules.inline.tag.exec(e2);
    if (t2) return !this.lexer.state.inLink && this.rules.other.startATag.test(t2[0]) ? this.lexer.state.inLink = true : this.lexer.state.inLink && this.rules.other.endATag.test(t2[0]) && (this.lexer.state.inLink = false), !this.lexer.state.inRawBlock && this.rules.other.startPreScriptTag.test(t2[0]) ? this.lexer.state.inRawBlock = true : this.lexer.state.inRawBlock && this.rules.other.endPreScriptTag.test(t2[0]) && (this.lexer.state.inRawBlock = false), { type: "html", raw: t2[0], inLink: this.lexer.state.inLink, inRawBlock: this.lexer.state.inRawBlock, block: false, text: t2[0] };
  }
  link(e2) {
    let t2 = this.rules.inline.link.exec(e2);
    if (t2) {
      let n2 = t2[2].trim();
      if (!this.options.pedantic && this.rules.other.startAngleBracket.test(n2)) {
        if (!this.rules.other.endAngleBracket.test(n2)) return;
        let s2 = $$1(n2.slice(0, -1), "\\");
        if ((n2.length - s2.length) % 2 === 0) return;
      } else {
        let s2 = ge(t2[2], "()");
        if (s2 === -2) return;
        if (s2 > -1) {
          let o2 = (t2[0].indexOf("!") === 0 ? 5 : 4) + t2[1].length + s2;
          t2[2] = t2[2].substring(0, s2), t2[0] = t2[0].substring(0, o2).trim(), t2[3] = "";
        }
      }
      let r2 = t2[2], i2 = "";
      if (this.options.pedantic) {
        let s2 = this.rules.other.pedanticHrefTitle.exec(r2);
        s2 && (r2 = s2[1], i2 = s2[3]);
      } else i2 = t2[3] ? t2[3].slice(1, -1) : "";
      return r2 = r2.trim(), this.rules.other.startAngleBracket.test(r2) && (this.options.pedantic && !this.rules.other.endAngleBracket.test(n2) ? r2 = r2.slice(1) : r2 = r2.slice(1, -1)), me(t2, { href: r2 && r2.replace(this.rules.inline.anyPunctuation, "$1"), title: i2 && i2.replace(this.rules.inline.anyPunctuation, "$1") }, t2[0], this.lexer, this.rules);
    }
  }
  reflink(e2, t2) {
    let n2;
    if ((n2 = this.rules.inline.reflink.exec(e2)) || (n2 = this.rules.inline.nolink.exec(e2))) {
      let r2 = (n2[2] || n2[1]).replace(this.rules.other.multipleSpaceGlobal, " "), i2 = t2[r2.toLowerCase()];
      if (!i2) {
        let s2 = n2[0].charAt(0);
        return { type: "text", raw: s2, text: s2 };
      }
      return me(n2, i2, n2[0], this.lexer, this.rules);
    }
  }
  emStrong(e2, t2, n2 = "") {
    let r2 = this.rules.inline.emStrongLDelim.exec(e2);
    if (!r2 || !r2[1] && !r2[2] && !r2[3] && !r2[4] || r2[4] && n2.match(this.rules.other.unicodeAlphaNumeric)) return;
    if (!(r2[1] || r2[3] || "") || !n2 || this.rules.inline.punctuation.exec(n2)) {
      let s2 = [...r2[0]].length - 1, a2, o2, l2 = s2, p2 = 0, c2 = r2[0][0] === "*" ? this.rules.inline.emStrongRDelimAst : this.rules.inline.emStrongRDelimUnd;
      for (c2.lastIndex = 0, t2 = t2.slice(-1 * e2.length + s2); (r2 = c2.exec(t2)) !== null; ) {
        if (a2 = r2[1] || r2[2] || r2[3] || r2[4] || r2[5] || r2[6], !a2) continue;
        if (o2 = [...a2].length, r2[3] || r2[4]) {
          l2 += o2;
          continue;
        } else if ((r2[5] || r2[6]) && s2 % 3 && !((s2 + o2) % 3)) {
          p2 += o2;
          continue;
        }
        if (l2 -= o2, l2 > 0) continue;
        o2 = Math.min(o2, o2 + l2 + p2);
        let d2 = [...r2[0]][0].length, h2 = e2.slice(0, s2 + r2.index + d2 + o2);
        if (Math.min(s2, o2) % 2) {
          let f2 = h2.slice(1, -1);
          return { type: "em", raw: h2, text: f2, tokens: this.lexer.inlineTokens(f2) };
        }
        let R2 = h2.slice(2, -2);
        return { type: "strong", raw: h2, text: R2, tokens: this.lexer.inlineTokens(R2) };
      }
    }
  }
  codespan(e2) {
    let t2 = this.rules.inline.code.exec(e2);
    if (t2) {
      let n2 = t2[2].replace(this.rules.other.newLineCharGlobal, " "), r2 = this.rules.other.nonSpaceChar.test(n2), i2 = this.rules.other.startingSpaceChar.test(n2) && this.rules.other.endingSpaceChar.test(n2);
      return r2 && i2 && (n2 = n2.substring(1, n2.length - 1)), { type: "codespan", raw: t2[0], text: n2 };
    }
  }
  br(e2) {
    let t2 = this.rules.inline.br.exec(e2);
    if (t2) return { type: "br", raw: t2[0] };
  }
  del(e2, t2, n2 = "") {
    let r2 = this.rules.inline.delLDelim.exec(e2);
    if (!r2) return;
    if (!(r2[1] || "") || !n2 || this.rules.inline.punctuation.exec(n2)) {
      let s2 = [...r2[0]].length - 1, a2, o2, l2 = s2, p2 = this.rules.inline.delRDelim;
      for (p2.lastIndex = 0, t2 = t2.slice(-1 * e2.length + s2); (r2 = p2.exec(t2)) !== null; ) {
        if (a2 = r2[1] || r2[2] || r2[3] || r2[4] || r2[5] || r2[6], !a2 || (o2 = [...a2].length, o2 !== s2)) continue;
        if (r2[3] || r2[4]) {
          l2 += o2;
          continue;
        }
        if (l2 -= o2, l2 > 0) continue;
        o2 = Math.min(o2, o2 + l2);
        let c2 = [...r2[0]][0].length, d2 = e2.slice(0, s2 + r2.index + c2 + o2), h2 = d2.slice(s2, -s2);
        return { type: "del", raw: d2, text: h2, tokens: this.lexer.inlineTokens(h2) };
      }
    }
  }
  autolink(e2) {
    let t2 = this.rules.inline.autolink.exec(e2);
    if (t2) {
      let n2, r2;
      return t2[2] === "@" ? (n2 = t2[1], r2 = "mailto:" + n2) : (n2 = t2[1], r2 = n2), { type: "link", raw: t2[0], text: n2, href: r2, tokens: [{ type: "text", raw: n2, text: n2 }] };
    }
  }
  url(e2) {
    var _a2;
    let t2;
    if (t2 = this.rules.inline.url.exec(e2)) {
      let n2, r2;
      if (t2[2] === "@") n2 = t2[0], r2 = "mailto:" + n2;
      else {
        let i2;
        do
          i2 = t2[0], t2[0] = ((_a2 = this.rules.inline._backpedal.exec(t2[0])) == null ? void 0 : _a2[0]) ?? "";
        while (i2 !== t2[0]);
        n2 = t2[0], t2[1] === "www." ? r2 = "http://" + t2[0] : r2 = t2[0];
      }
      return { type: "link", raw: t2[0], text: n2, href: r2, tokens: [{ type: "text", raw: n2, text: n2 }] };
    }
  }
  inlineText(e2) {
    let t2 = this.rules.inline.text.exec(e2);
    if (t2) {
      let n2 = this.lexer.state.inRawBlock;
      return { type: "text", raw: t2[0], text: t2[0], escaped: n2 };
    }
  }
};
var x$2 = class u {
  constructor(e2) {
    __publicField(this, "tokens");
    __publicField(this, "options");
    __publicField(this, "state");
    __publicField(this, "inlineQueue");
    __publicField(this, "tokenizer");
    this.tokens = [], this.tokens.links = /* @__PURE__ */ Object.create(null), this.options = e2 || T$1, this.options.tokenizer = this.options.tokenizer || new w$3(), this.tokenizer = this.options.tokenizer, this.tokenizer.options = this.options, this.tokenizer.lexer = this, this.inlineQueue = [], this.state = { inLink: false, inRawBlock: false, top: true };
    let t2 = { other: m$2, block: B$1.normal, inline: I.normal };
    this.options.pedantic ? (t2.block = B$1.pedantic, t2.inline = I.pedantic) : this.options.gfm && (t2.block = B$1.gfm, this.options.breaks ? t2.inline = I.breaks : t2.inline = I.gfm), this.tokenizer.rules = t2;
  }
  static get rules() {
    return { block: B$1, inline: I };
  }
  static lex(e2, t2) {
    return new u(t2).lex(e2);
  }
  static lexInline(e2, t2) {
    return new u(t2).inlineTokens(e2);
  }
  lex(e2) {
    e2 = e2.replace(m$2.carriageReturn, `
`), this.blockTokens(e2, this.tokens);
    for (let t2 = 0; t2 < this.inlineQueue.length; t2++) {
      let n2 = this.inlineQueue[t2];
      this.inlineTokens(n2.src, n2.tokens);
    }
    return this.inlineQueue = [], this.tokens;
  }
  blockTokens(e2, t2 = [], n2 = false) {
    var _a2, _b, _c;
    for (this.tokenizer.lexer = this, this.options.pedantic && (e2 = e2.replace(m$2.tabCharGlobal, "    ").replace(m$2.spaceLine, "")); e2; ) {
      let r2;
      if ((_b = (_a2 = this.options.extensions) == null ? void 0 : _a2.block) == null ? void 0 : _b.some((s2) => (r2 = s2.call({ lexer: this }, e2, t2)) ? (e2 = e2.substring(r2.raw.length), t2.push(r2), true) : false)) continue;
      if (r2 = this.tokenizer.space(e2)) {
        e2 = e2.substring(r2.raw.length);
        let s2 = t2.at(-1);
        r2.raw.length === 1 && s2 !== void 0 ? s2.raw += `
` : t2.push(r2);
        continue;
      }
      if (r2 = this.tokenizer.code(e2)) {
        e2 = e2.substring(r2.raw.length);
        let s2 = t2.at(-1);
        (s2 == null ? void 0 : s2.type) === "paragraph" || (s2 == null ? void 0 : s2.type) === "text" ? (s2.raw += (s2.raw.endsWith(`
`) ? "" : `
`) + r2.raw, s2.text += `
` + r2.text, this.inlineQueue.at(-1).src = s2.text) : t2.push(r2);
        continue;
      }
      if (r2 = this.tokenizer.fences(e2)) {
        e2 = e2.substring(r2.raw.length), t2.push(r2);
        continue;
      }
      if (r2 = this.tokenizer.heading(e2)) {
        e2 = e2.substring(r2.raw.length), t2.push(r2);
        continue;
      }
      if (r2 = this.tokenizer.hr(e2)) {
        e2 = e2.substring(r2.raw.length), t2.push(r2);
        continue;
      }
      if (r2 = this.tokenizer.blockquote(e2)) {
        e2 = e2.substring(r2.raw.length), t2.push(r2);
        continue;
      }
      if (r2 = this.tokenizer.list(e2)) {
        e2 = e2.substring(r2.raw.length), t2.push(r2);
        continue;
      }
      if (r2 = this.tokenizer.html(e2)) {
        e2 = e2.substring(r2.raw.length), t2.push(r2);
        continue;
      }
      if (r2 = this.tokenizer.def(e2)) {
        e2 = e2.substring(r2.raw.length);
        let s2 = t2.at(-1);
        (s2 == null ? void 0 : s2.type) === "paragraph" || (s2 == null ? void 0 : s2.type) === "text" ? (s2.raw += (s2.raw.endsWith(`
`) ? "" : `
`) + r2.raw, s2.text += `
` + r2.raw, this.inlineQueue.at(-1).src = s2.text) : this.tokens.links[r2.tag] || (this.tokens.links[r2.tag] = { href: r2.href, title: r2.title }, t2.push(r2));
        continue;
      }
      if (r2 = this.tokenizer.table(e2)) {
        e2 = e2.substring(r2.raw.length), t2.push(r2);
        continue;
      }
      if (r2 = this.tokenizer.lheading(e2)) {
        e2 = e2.substring(r2.raw.length), t2.push(r2);
        continue;
      }
      let i2 = e2;
      if ((_c = this.options.extensions) == null ? void 0 : _c.startBlock) {
        let s2 = 1 / 0, a2 = e2.slice(1), o2;
        this.options.extensions.startBlock.forEach((l2) => {
          o2 = l2.call({ lexer: this }, a2), typeof o2 == "number" && o2 >= 0 && (s2 = Math.min(s2, o2));
        }), s2 < 1 / 0 && s2 >= 0 && (i2 = e2.substring(0, s2 + 1));
      }
      if (this.state.top && (r2 = this.tokenizer.paragraph(i2))) {
        let s2 = t2.at(-1);
        n2 && (s2 == null ? void 0 : s2.type) === "paragraph" ? (s2.raw += (s2.raw.endsWith(`
`) ? "" : `
`) + r2.raw, s2.text += `
` + r2.text, this.inlineQueue.pop(), this.inlineQueue.at(-1).src = s2.text) : t2.push(r2), n2 = i2.length !== e2.length, e2 = e2.substring(r2.raw.length);
        continue;
      }
      if (r2 = this.tokenizer.text(e2)) {
        e2 = e2.substring(r2.raw.length);
        let s2 = t2.at(-1);
        (s2 == null ? void 0 : s2.type) === "text" ? (s2.raw += (s2.raw.endsWith(`
`) ? "" : `
`) + r2.raw, s2.text += `
` + r2.text, this.inlineQueue.pop(), this.inlineQueue.at(-1).src = s2.text) : t2.push(r2);
        continue;
      }
      if (e2) {
        let s2 = "Infinite loop on byte: " + e2.charCodeAt(0);
        if (this.options.silent) {
          console.error(s2);
          break;
        } else throw new Error(s2);
      }
    }
    return this.state.top = true, t2;
  }
  inline(e2, t2 = []) {
    return this.inlineQueue.push({ src: e2, tokens: t2 }), t2;
  }
  inlineTokens(e2, t2 = []) {
    var _a2, _b, _c, _d, _e2;
    this.tokenizer.lexer = this;
    let n2 = e2, r2 = null;
    if (this.tokens.links) {
      let o2 = Object.keys(this.tokens.links);
      if (o2.length > 0) for (; (r2 = this.tokenizer.rules.inline.reflinkSearch.exec(n2)) !== null; ) o2.includes(r2[0].slice(r2[0].lastIndexOf("[") + 1, -1)) && (n2 = n2.slice(0, r2.index) + "[" + "a".repeat(r2[0].length - 2) + "]" + n2.slice(this.tokenizer.rules.inline.reflinkSearch.lastIndex));
    }
    for (; (r2 = this.tokenizer.rules.inline.anyPunctuation.exec(n2)) !== null; ) n2 = n2.slice(0, r2.index) + "++" + n2.slice(this.tokenizer.rules.inline.anyPunctuation.lastIndex);
    let i2;
    for (; (r2 = this.tokenizer.rules.inline.blockSkip.exec(n2)) !== null; ) i2 = r2[2] ? r2[2].length : 0, n2 = n2.slice(0, r2.index + i2) + "[" + "a".repeat(r2[0].length - i2 - 2) + "]" + n2.slice(this.tokenizer.rules.inline.blockSkip.lastIndex);
    n2 = ((_b = (_a2 = this.options.hooks) == null ? void 0 : _a2.emStrongMask) == null ? void 0 : _b.call({ lexer: this }, n2)) ?? n2;
    let s2 = false, a2 = "";
    for (; e2; ) {
      s2 || (a2 = ""), s2 = false;
      let o2;
      if ((_d = (_c = this.options.extensions) == null ? void 0 : _c.inline) == null ? void 0 : _d.some((p2) => (o2 = p2.call({ lexer: this }, e2, t2)) ? (e2 = e2.substring(o2.raw.length), t2.push(o2), true) : false)) continue;
      if (o2 = this.tokenizer.escape(e2)) {
        e2 = e2.substring(o2.raw.length), t2.push(o2);
        continue;
      }
      if (o2 = this.tokenizer.tag(e2)) {
        e2 = e2.substring(o2.raw.length), t2.push(o2);
        continue;
      }
      if (o2 = this.tokenizer.link(e2)) {
        e2 = e2.substring(o2.raw.length), t2.push(o2);
        continue;
      }
      if (o2 = this.tokenizer.reflink(e2, this.tokens.links)) {
        e2 = e2.substring(o2.raw.length);
        let p2 = t2.at(-1);
        o2.type === "text" && (p2 == null ? void 0 : p2.type) === "text" ? (p2.raw += o2.raw, p2.text += o2.text) : t2.push(o2);
        continue;
      }
      if (o2 = this.tokenizer.emStrong(e2, n2, a2)) {
        e2 = e2.substring(o2.raw.length), t2.push(o2);
        continue;
      }
      if (o2 = this.tokenizer.codespan(e2)) {
        e2 = e2.substring(o2.raw.length), t2.push(o2);
        continue;
      }
      if (o2 = this.tokenizer.br(e2)) {
        e2 = e2.substring(o2.raw.length), t2.push(o2);
        continue;
      }
      if (o2 = this.tokenizer.del(e2, n2, a2)) {
        e2 = e2.substring(o2.raw.length), t2.push(o2);
        continue;
      }
      if (o2 = this.tokenizer.autolink(e2)) {
        e2 = e2.substring(o2.raw.length), t2.push(o2);
        continue;
      }
      if (!this.state.inLink && (o2 = this.tokenizer.url(e2))) {
        e2 = e2.substring(o2.raw.length), t2.push(o2);
        continue;
      }
      let l2 = e2;
      if ((_e2 = this.options.extensions) == null ? void 0 : _e2.startInline) {
        let p2 = 1 / 0, c2 = e2.slice(1), d2;
        this.options.extensions.startInline.forEach((h2) => {
          d2 = h2.call({ lexer: this }, c2), typeof d2 == "number" && d2 >= 0 && (p2 = Math.min(p2, d2));
        }), p2 < 1 / 0 && p2 >= 0 && (l2 = e2.substring(0, p2 + 1));
      }
      if (o2 = this.tokenizer.inlineText(l2)) {
        e2 = e2.substring(o2.raw.length), o2.raw.slice(-1) !== "_" && (a2 = o2.raw.slice(-1)), s2 = true;
        let p2 = t2.at(-1);
        (p2 == null ? void 0 : p2.type) === "text" ? (p2.raw += o2.raw, p2.text += o2.text) : t2.push(o2);
        continue;
      }
      if (e2) {
        let p2 = "Infinite loop on byte: " + e2.charCodeAt(0);
        if (this.options.silent) {
          console.error(p2);
          break;
        } else throw new Error(p2);
      }
    }
    return t2;
  }
};
var y$3 = class y {
  constructor(e2) {
    __publicField(this, "options");
    __publicField(this, "parser");
    this.options = e2 || T$1;
  }
  space(e2) {
    return "";
  }
  code({ text: e2, lang: t2, escaped: n2 }) {
    var _a2;
    let r2 = (_a2 = (t2 || "").match(m$2.notSpaceStart)) == null ? void 0 : _a2[0], i2 = e2.replace(m$2.endingNewline, "") + `
`;
    return r2 ? '<pre><code class="language-' + O(r2) + '">' + (n2 ? i2 : O(i2, true)) + `</code></pre>
` : "<pre><code>" + (n2 ? i2 : O(i2, true)) + `</code></pre>
`;
  }
  blockquote({ tokens: e2 }) {
    return `<blockquote>
${this.parser.parse(e2)}</blockquote>
`;
  }
  html({ text: e2 }) {
    return e2;
  }
  def(e2) {
    return "";
  }
  heading({ tokens: e2, depth: t2 }) {
    return `<h${t2}>${this.parser.parseInline(e2)}</h${t2}>
`;
  }
  hr(e2) {
    return `<hr>
`;
  }
  list(e2) {
    let t2 = e2.ordered, n2 = e2.start, r2 = "";
    for (let a2 = 0; a2 < e2.items.length; a2++) {
      let o2 = e2.items[a2];
      r2 += this.listitem(o2);
    }
    let i2 = t2 ? "ol" : "ul", s2 = t2 && n2 !== 1 ? ' start="' + n2 + '"' : "";
    return "<" + i2 + s2 + `>
` + r2 + "</" + i2 + `>
`;
  }
  listitem(e2) {
    return `<li>${this.parser.parse(e2.tokens)}</li>
`;
  }
  checkbox({ checked: e2 }) {
    return "<input " + (e2 ? 'checked="" ' : "") + 'disabled="" type="checkbox"> ';
  }
  paragraph({ tokens: e2 }) {
    return `<p>${this.parser.parseInline(e2)}</p>
`;
  }
  table(e2) {
    let t2 = "", n2 = "";
    for (let i2 = 0; i2 < e2.header.length; i2++) n2 += this.tablecell(e2.header[i2]);
    t2 += this.tablerow({ text: n2 });
    let r2 = "";
    for (let i2 = 0; i2 < e2.rows.length; i2++) {
      let s2 = e2.rows[i2];
      n2 = "";
      for (let a2 = 0; a2 < s2.length; a2++) n2 += this.tablecell(s2[a2]);
      r2 += this.tablerow({ text: n2 });
    }
    return r2 && (r2 = `<tbody>${r2}</tbody>`), `<table>
<thead>
` + t2 + `</thead>
` + r2 + `</table>
`;
  }
  tablerow({ text: e2 }) {
    return `<tr>
${e2}</tr>
`;
  }
  tablecell(e2) {
    let t2 = this.parser.parseInline(e2.tokens), n2 = e2.header ? "th" : "td";
    return (e2.align ? `<${n2} align="${e2.align}">` : `<${n2}>`) + t2 + `</${n2}>
`;
  }
  strong({ tokens: e2 }) {
    return `<strong>${this.parser.parseInline(e2)}</strong>`;
  }
  em({ tokens: e2 }) {
    return `<em>${this.parser.parseInline(e2)}</em>`;
  }
  codespan({ text: e2 }) {
    return `<code>${O(e2, true)}</code>`;
  }
  br(e2) {
    return "<br>";
  }
  del({ tokens: e2 }) {
    return `<del>${this.parser.parseInline(e2)}</del>`;
  }
  link({ href: e2, title: t2, tokens: n2 }) {
    let r2 = this.parser.parseInline(n2), i2 = J(e2);
    if (i2 === null) return r2;
    e2 = i2;
    let s2 = '<a href="' + e2 + '"';
    return t2 && (s2 += ' title="' + O(t2) + '"'), s2 += ">" + r2 + "</a>", s2;
  }
  image({ href: e2, title: t2, text: n2, tokens: r2 }) {
    r2 && (n2 = this.parser.parseInline(r2, this.parser.textRenderer));
    let i2 = J(e2);
    if (i2 === null) return O(n2);
    e2 = i2;
    let s2 = `<img src="${e2}" alt="${O(n2)}"`;
    return t2 && (s2 += ` title="${O(t2)}"`), s2 += ">", s2;
  }
  text(e2) {
    return "tokens" in e2 && e2.tokens ? this.parser.parseInline(e2.tokens) : "escaped" in e2 && e2.escaped ? e2.text : O(e2.text);
  }
};
var L = class {
  strong({ text: e2 }) {
    return e2;
  }
  em({ text: e2 }) {
    return e2;
  }
  codespan({ text: e2 }) {
    return e2;
  }
  del({ text: e2 }) {
    return e2;
  }
  html({ text: e2 }) {
    return e2;
  }
  text({ text: e2 }) {
    return e2;
  }
  link({ text: e2 }) {
    return "" + e2;
  }
  image({ text: e2 }) {
    return "" + e2;
  }
  br() {
    return "";
  }
  checkbox({ raw: e2 }) {
    return e2;
  }
};
var b$2 = class u2 {
  constructor(e2) {
    __publicField(this, "options");
    __publicField(this, "renderer");
    __publicField(this, "textRenderer");
    this.options = e2 || T$1, this.options.renderer = this.options.renderer || new y$3(), this.renderer = this.options.renderer, this.renderer.options = this.options, this.renderer.parser = this, this.textRenderer = new L();
  }
  static parse(e2, t2) {
    return new u2(t2).parse(e2);
  }
  static parseInline(e2, t2) {
    return new u2(t2).parseInline(e2);
  }
  parse(e2) {
    var _a2, _b;
    this.renderer.parser = this;
    let t2 = "";
    for (let n2 = 0; n2 < e2.length; n2++) {
      let r2 = e2[n2];
      if ((_b = (_a2 = this.options.extensions) == null ? void 0 : _a2.renderers) == null ? void 0 : _b[r2.type]) {
        let s2 = r2, a2 = this.options.extensions.renderers[s2.type].call({ parser: this }, s2);
        if (a2 !== false || !["space", "hr", "heading", "code", "table", "blockquote", "list", "html", "def", "paragraph", "text"].includes(s2.type)) {
          t2 += a2 || "";
          continue;
        }
      }
      let i2 = r2;
      switch (i2.type) {
        case "space": {
          t2 += this.renderer.space(i2);
          break;
        }
        case "hr": {
          t2 += this.renderer.hr(i2);
          break;
        }
        case "heading": {
          t2 += this.renderer.heading(i2);
          break;
        }
        case "code": {
          t2 += this.renderer.code(i2);
          break;
        }
        case "table": {
          t2 += this.renderer.table(i2);
          break;
        }
        case "blockquote": {
          t2 += this.renderer.blockquote(i2);
          break;
        }
        case "list": {
          t2 += this.renderer.list(i2);
          break;
        }
        case "checkbox": {
          t2 += this.renderer.checkbox(i2);
          break;
        }
        case "html": {
          t2 += this.renderer.html(i2);
          break;
        }
        case "def": {
          t2 += this.renderer.def(i2);
          break;
        }
        case "paragraph": {
          t2 += this.renderer.paragraph(i2);
          break;
        }
        case "text": {
          t2 += this.renderer.text(i2);
          break;
        }
        default: {
          let s2 = 'Token with "' + i2.type + '" type was not found.';
          if (this.options.silent) return console.error(s2), "";
          throw new Error(s2);
        }
      }
    }
    return t2;
  }
  parseInline(e2, t2 = this.renderer) {
    var _a2, _b;
    this.renderer.parser = this;
    let n2 = "";
    for (let r2 = 0; r2 < e2.length; r2++) {
      let i2 = e2[r2];
      if ((_b = (_a2 = this.options.extensions) == null ? void 0 : _a2.renderers) == null ? void 0 : _b[i2.type]) {
        let a2 = this.options.extensions.renderers[i2.type].call({ parser: this }, i2);
        if (a2 !== false || !["escape", "html", "link", "image", "strong", "em", "codespan", "br", "del", "text"].includes(i2.type)) {
          n2 += a2 || "";
          continue;
        }
      }
      let s2 = i2;
      switch (s2.type) {
        case "escape": {
          n2 += t2.text(s2);
          break;
        }
        case "html": {
          n2 += t2.html(s2);
          break;
        }
        case "link": {
          n2 += t2.link(s2);
          break;
        }
        case "image": {
          n2 += t2.image(s2);
          break;
        }
        case "checkbox": {
          n2 += t2.checkbox(s2);
          break;
        }
        case "strong": {
          n2 += t2.strong(s2);
          break;
        }
        case "em": {
          n2 += t2.em(s2);
          break;
        }
        case "codespan": {
          n2 += t2.codespan(s2);
          break;
        }
        case "br": {
          n2 += t2.br(s2);
          break;
        }
        case "del": {
          n2 += t2.del(s2);
          break;
        }
        case "text": {
          n2 += t2.text(s2);
          break;
        }
        default: {
          let a2 = 'Token with "' + s2.type + '" type was not found.';
          if (this.options.silent) return console.error(a2), "";
          throw new Error(a2);
        }
      }
    }
    return n2;
  }
};
var P = (_a = class {
  constructor(e2) {
    __publicField(this, "options");
    __publicField(this, "block");
    this.options = e2 || T$1;
  }
  preprocess(e2) {
    return e2;
  }
  postprocess(e2) {
    return e2;
  }
  processAllTokens(e2) {
    return e2;
  }
  emStrongMask(e2) {
    return e2;
  }
  provideLexer(e2 = this.block) {
    return e2 ? x$2.lex : x$2.lexInline;
  }
  provideParser(e2 = this.block) {
    return e2 ? b$2.parse : b$2.parseInline;
  }
}, __publicField(_a, "passThroughHooks", /* @__PURE__ */ new Set(["preprocess", "postprocess", "processAllTokens", "emStrongMask"])), __publicField(_a, "passThroughHooksRespectAsync", /* @__PURE__ */ new Set(["preprocess", "postprocess", "processAllTokens"])), _a);
var D$1 = class D {
  constructor(...e2) {
    __publicField(this, "defaults", z$1());
    __publicField(this, "options", this.setOptions);
    __publicField(this, "parse", this.parseMarkdown(true));
    __publicField(this, "parseInline", this.parseMarkdown(false));
    __publicField(this, "Parser", b$2);
    __publicField(this, "Renderer", y$3);
    __publicField(this, "TextRenderer", L);
    __publicField(this, "Lexer", x$2);
    __publicField(this, "Tokenizer", w$3);
    __publicField(this, "Hooks", P);
    this.use(...e2);
  }
  walkTokens(e2, t2) {
    var _a2, _b;
    let n2 = [];
    for (let r2 of e2) switch (n2 = n2.concat(t2.call(this, r2)), r2.type) {
      case "table": {
        let i2 = r2;
        for (let s2 of i2.header) n2 = n2.concat(this.walkTokens(s2.tokens, t2));
        for (let s2 of i2.rows) for (let a2 of s2) n2 = n2.concat(this.walkTokens(a2.tokens, t2));
        break;
      }
      case "list": {
        let i2 = r2;
        n2 = n2.concat(this.walkTokens(i2.items, t2));
        break;
      }
      default: {
        let i2 = r2;
        ((_b = (_a2 = this.defaults.extensions) == null ? void 0 : _a2.childTokens) == null ? void 0 : _b[i2.type]) ? this.defaults.extensions.childTokens[i2.type].forEach((s2) => {
          let a2 = i2[s2].flat(1 / 0);
          n2 = n2.concat(this.walkTokens(a2, t2));
        }) : i2.tokens && (n2 = n2.concat(this.walkTokens(i2.tokens, t2)));
      }
    }
    return n2;
  }
  use(...e2) {
    let t2 = this.defaults.extensions || { renderers: {}, childTokens: {} };
    return e2.forEach((n2) => {
      let r2 = { ...n2 };
      if (r2.async = this.defaults.async || r2.async || false, n2.extensions && (n2.extensions.forEach((i2) => {
        if (!i2.name) throw new Error("extension name required");
        if ("renderer" in i2) {
          let s2 = t2.renderers[i2.name];
          s2 ? t2.renderers[i2.name] = function(...a2) {
            let o2 = i2.renderer.apply(this, a2);
            return o2 === false && (o2 = s2.apply(this, a2)), o2;
          } : t2.renderers[i2.name] = i2.renderer;
        }
        if ("tokenizer" in i2) {
          if (!i2.level || i2.level !== "block" && i2.level !== "inline") throw new Error("extension level must be 'block' or 'inline'");
          let s2 = t2[i2.level];
          s2 ? s2.unshift(i2.tokenizer) : t2[i2.level] = [i2.tokenizer], i2.start && (i2.level === "block" ? t2.startBlock ? t2.startBlock.push(i2.start) : t2.startBlock = [i2.start] : i2.level === "inline" && (t2.startInline ? t2.startInline.push(i2.start) : t2.startInline = [i2.start]));
        }
        "childTokens" in i2 && i2.childTokens && (t2.childTokens[i2.name] = i2.childTokens);
      }), r2.extensions = t2), n2.renderer) {
        let i2 = this.defaults.renderer || new y$3(this.defaults);
        for (let s2 in n2.renderer) {
          if (!(s2 in i2)) throw new Error(`renderer '${s2}' does not exist`);
          if (["options", "parser"].includes(s2)) continue;
          let a2 = s2, o2 = n2.renderer[a2], l2 = i2[a2];
          i2[a2] = (...p2) => {
            let c2 = o2.apply(i2, p2);
            return c2 === false && (c2 = l2.apply(i2, p2)), c2 || "";
          };
        }
        r2.renderer = i2;
      }
      if (n2.tokenizer) {
        let i2 = this.defaults.tokenizer || new w$3(this.defaults);
        for (let s2 in n2.tokenizer) {
          if (!(s2 in i2)) throw new Error(`tokenizer '${s2}' does not exist`);
          if (["options", "rules", "lexer"].includes(s2)) continue;
          let a2 = s2, o2 = n2.tokenizer[a2], l2 = i2[a2];
          i2[a2] = (...p2) => {
            let c2 = o2.apply(i2, p2);
            return c2 === false && (c2 = l2.apply(i2, p2)), c2;
          };
        }
        r2.tokenizer = i2;
      }
      if (n2.hooks) {
        let i2 = this.defaults.hooks || new P();
        for (let s2 in n2.hooks) {
          if (!(s2 in i2)) throw new Error(`hook '${s2}' does not exist`);
          if (["options", "block"].includes(s2)) continue;
          let a2 = s2, o2 = n2.hooks[a2], l2 = i2[a2];
          P.passThroughHooks.has(s2) ? i2[a2] = (p2) => {
            if (this.defaults.async && P.passThroughHooksRespectAsync.has(s2)) return (async () => {
              let d2 = await o2.call(i2, p2);
              return l2.call(i2, d2);
            })();
            let c2 = o2.call(i2, p2);
            return l2.call(i2, c2);
          } : i2[a2] = (...p2) => {
            if (this.defaults.async) return (async () => {
              let d2 = await o2.apply(i2, p2);
              return d2 === false && (d2 = await l2.apply(i2, p2)), d2;
            })();
            let c2 = o2.apply(i2, p2);
            return c2 === false && (c2 = l2.apply(i2, p2)), c2;
          };
        }
        r2.hooks = i2;
      }
      if (n2.walkTokens) {
        let i2 = this.defaults.walkTokens, s2 = n2.walkTokens;
        r2.walkTokens = function(a2) {
          let o2 = [];
          return o2.push(s2.call(this, a2)), i2 && (o2 = o2.concat(i2.call(this, a2))), o2;
        };
      }
      this.defaults = { ...this.defaults, ...r2 };
    }), this;
  }
  setOptions(e2) {
    return this.defaults = { ...this.defaults, ...e2 }, this;
  }
  lexer(e2, t2) {
    return x$2.lex(e2, t2 ?? this.defaults);
  }
  parser(e2, t2) {
    return b$2.parse(e2, t2 ?? this.defaults);
  }
  parseMarkdown(e2) {
    return (n2, r2) => {
      let i2 = { ...r2 }, s2 = { ...this.defaults, ...i2 }, a2 = this.onError(!!s2.silent, !!s2.async);
      if (this.defaults.async === true && i2.async === false) return a2(new Error("marked(): The async option was set to true by an extension. Remove async: false from the parse options object to return a Promise."));
      if (typeof n2 > "u" || n2 === null) return a2(new Error("marked(): input parameter is undefined or null"));
      if (typeof n2 != "string") return a2(new Error("marked(): input parameter is of type " + Object.prototype.toString.call(n2) + ", string expected"));
      if (s2.hooks && (s2.hooks.options = s2, s2.hooks.block = e2), s2.async) return (async () => {
        let o2 = s2.hooks ? await s2.hooks.preprocess(n2) : n2, p2 = await (s2.hooks ? await s2.hooks.provideLexer(e2) : e2 ? x$2.lex : x$2.lexInline)(o2, s2), c2 = s2.hooks ? await s2.hooks.processAllTokens(p2) : p2;
        s2.walkTokens && await Promise.all(this.walkTokens(c2, s2.walkTokens));
        let h2 = await (s2.hooks ? await s2.hooks.provideParser(e2) : e2 ? b$2.parse : b$2.parseInline)(c2, s2);
        return s2.hooks ? await s2.hooks.postprocess(h2) : h2;
      })().catch(a2);
      try {
        s2.hooks && (n2 = s2.hooks.preprocess(n2));
        let l2 = (s2.hooks ? s2.hooks.provideLexer(e2) : e2 ? x$2.lex : x$2.lexInline)(n2, s2);
        s2.hooks && (l2 = s2.hooks.processAllTokens(l2)), s2.walkTokens && this.walkTokens(l2, s2.walkTokens);
        let c2 = (s2.hooks ? s2.hooks.provideParser(e2) : e2 ? b$2.parse : b$2.parseInline)(l2, s2);
        return s2.hooks && (c2 = s2.hooks.postprocess(c2)), c2;
      } catch (o2) {
        return a2(o2);
      }
    };
  }
  onError(e2, t2) {
    return (n2) => {
      if (n2.message += `
Please report this to https://github.com/markedjs/marked.`, e2) {
        let r2 = "<p>An error occurred:</p><pre>" + O(n2.message + "", true) + "</pre>";
        return t2 ? Promise.resolve(r2) : r2;
      }
      if (t2) return Promise.reject(n2);
      throw n2;
    };
  }
};
var M = new D$1();
function g$2(u4, e2) {
  return M.parse(u4, e2);
}
g$2.options = g$2.setOptions = function(u4) {
  return M.setOptions(u4), g$2.defaults = M.defaults, G(g$2.defaults), g$2;
};
g$2.getDefaults = z$1;
g$2.defaults = T$1;
g$2.use = function(...u4) {
  return M.use(...u4), g$2.defaults = M.defaults, G(g$2.defaults), g$2;
};
g$2.walkTokens = function(u4, e2) {
  return M.walkTokens(u4, e2);
};
g$2.parseInline = M.parseInline;
g$2.Parser = b$2;
g$2.parser = b$2.parse;
g$2.Renderer = y$3;
g$2.TextRenderer = L;
g$2.Lexer = x$2;
g$2.lexer = x$2.lex;
g$2.Tokenizer = w$3;
g$2.Hooks = P;
g$2.parse = g$2;
g$2.options;
g$2.setOptions;
g$2.use;
g$2.walkTokens;
g$2.parseInline;
b$2.parse;
x$2.lex;
const scriptRel = "modulepreload";
const assetsURL = function(dep) {
  return "/" + dep;
};
const seen = {};
const __vitePreload = function preload(baseModule, deps, importerUrl) {
  let promise = Promise.resolve();
  if (deps && deps.length > 0) {
    let allSettled2 = function(promises) {
      return Promise.all(
        promises.map(
          (p2) => Promise.resolve(p2).then(
            (value) => ({ status: "fulfilled", value }),
            (reason) => ({ status: "rejected", reason })
          )
        )
      );
    };
    document.getElementsByTagName("link");
    const cspNonceMeta = document.querySelector(
      "meta[property=csp-nonce]"
    );
    const cspNonce = (cspNonceMeta == null ? void 0 : cspNonceMeta.nonce) || (cspNonceMeta == null ? void 0 : cspNonceMeta.getAttribute("nonce"));
    promise = allSettled2(
      deps.map((dep) => {
        dep = assetsURL(dep);
        if (dep in seen) return;
        seen[dep] = true;
        const isCss = dep.endsWith(".css");
        const cssSelector = isCss ? '[rel="stylesheet"]' : "";
        if (document.querySelector(`link[href="${dep}"]${cssSelector}`)) {
          return;
        }
        const link = document.createElement("link");
        link.rel = isCss ? "stylesheet" : scriptRel;
        if (!isCss) {
          link.as = "script";
        }
        link.crossOrigin = "";
        link.href = dep;
        if (cspNonce) {
          link.setAttribute("nonce", cspNonce);
        }
        document.head.appendChild(link);
        if (isCss) {
          return new Promise((res, rej) => {
            link.addEventListener("load", res);
            link.addEventListener(
              "error",
              () => rej(new Error(`Unable to preload CSS for ${dep}`))
            );
          });
        }
      })
    );
  }
  function handlePreloadError(err) {
    const e2 = new Event("vite:preloadError", {
      cancelable: true
    });
    e2.payload = err;
    window.dispatchEvent(e2);
    if (!e2.defaultPrevented) {
      throw err;
    }
  }
  return promise.then((res) => {
    for (const item of res || []) {
      if (item.status !== "rejected") continue;
      handlePreloadError(item.reason);
    }
    return baseModule().catch(handlePreloadError);
  });
};
const __variableDynamicImportRuntimeHelper = (glob, path, segs) => {
  const v2 = glob[path];
  if (v2) {
    return typeof v2 === "function" ? v2() : Promise.resolve(v2);
  }
  return new Promise((_2, reject) => {
    (typeof queueMicrotask === "function" ? queueMicrotask : setTimeout)(
      reject.bind(
        null,
        new Error(
          "Unknown variable dynamic import: " + path + (path.split("/").length !== segs ? ". Note that variables only represent file names one level deep." : "")
        )
      )
    );
  });
};
var t$2, r$1, u$1, i$1, o$1 = 0, f = [], c$1 = l$3, e$1 = c$1.__b, a$1 = c$1.__r, v$1 = c$1.diffed, l$2 = c$1.__c, m$1 = c$1.unmount, s$1 = c$1.__;
function p$2(n2, t2) {
  c$1.__h && c$1.__h(r$1, n2, o$1 || t2), o$1 = 0;
  var u4 = r$1.__H || (r$1.__H = { __: [], __h: [] });
  return n2 >= u4.__.length && u4.__.push({}), u4.__[n2];
}
function d$2(n2) {
  return o$1 = 1, h$2(D2, n2);
}
function h$2(n2, u4, i2) {
  var o2 = p$2(t$2++, 2);
  if (o2.t = n2, !o2.__c && (o2.__ = [D2(void 0, u4), function(n3) {
    var t2 = o2.__N ? o2.__N[0] : o2.__[0], r2 = o2.t(t2, n3);
    t2 !== r2 && (o2.__N = [r2, o2.__[1]], o2.__c.setState({}));
  }], o2.__c = r$1, !r$1.__f)) {
    var f2 = function(n3, t2, r2) {
      if (!o2.__c.__H) return true;
      var u5 = o2.__c.__H.__.filter(function(n4) {
        return n4.__c;
      });
      if (u5.every(function(n4) {
        return !n4.__N;
      })) return !c2 || c2.call(this, n3, t2, r2);
      var i3 = o2.__c.props !== n3;
      return u5.some(function(n4) {
        if (n4.__N) {
          var t3 = n4.__[0];
          n4.__ = n4.__N, n4.__N = void 0, t3 !== n4.__[0] && (i3 = true);
        }
      }), c2 && c2.call(this, n3, t2, r2) || i3;
    };
    r$1.__f = true;
    var c2 = r$1.shouldComponentUpdate, e2 = r$1.componentWillUpdate;
    r$1.componentWillUpdate = function(n3, t2, r2) {
      if (this.__e) {
        var u5 = c2;
        c2 = void 0, f2(n3, t2, r2), c2 = u5;
      }
      e2 && e2.call(this, n3, t2, r2);
    }, r$1.shouldComponentUpdate = f2;
  }
  return o2.__N || o2.__;
}
function y$2(n2, u4) {
  var i2 = p$2(t$2++, 3);
  !c$1.__s && C(i2.__H, u4) && (i2.__ = n2, i2.u = u4, r$1.__H.__h.push(i2));
}
function A(n2) {
  return o$1 = 5, T(function() {
    return { current: n2 };
  }, []);
}
function T(n2, r2) {
  var u4 = p$2(t$2++, 7);
  return C(u4.__H, r2) && (u4.__ = n2(), u4.__H = r2, u4.__h = n2), u4.__;
}
function q$1(n2, t2) {
  return o$1 = 8, T(function() {
    return n2;
  }, t2);
}
function j$1() {
  for (var n2; n2 = f.shift(); ) {
    var t2 = n2.__H;
    if (n2.__P && t2) try {
      t2.__h.some(z), t2.__h.some(B), t2.__h = [];
    } catch (r2) {
      t2.__h = [], c$1.__e(r2, n2.__v);
    }
  }
}
c$1.__b = function(n2) {
  r$1 = null, e$1 && e$1(n2);
}, c$1.__ = function(n2, t2) {
  n2 && t2.__k && t2.__k.__m && (n2.__m = t2.__k.__m), s$1 && s$1(n2, t2);
}, c$1.__r = function(n2) {
  a$1 && a$1(n2), t$2 = 0;
  var i2 = (r$1 = n2.__c).__H;
  i2 && (u$1 === r$1 ? (i2.__h = [], r$1.__h = [], i2.__.some(function(n3) {
    n3.__N && (n3.__ = n3.__N), n3.u = n3.__N = void 0;
  })) : (i2.__h.some(z), i2.__h.some(B), i2.__h = [], t$2 = 0)), u$1 = r$1;
}, c$1.diffed = function(n2) {
  v$1 && v$1(n2);
  var t2 = n2.__c;
  t2 && t2.__H && (t2.__H.__h.length && (1 !== f.push(t2) && i$1 === c$1.requestAnimationFrame || ((i$1 = c$1.requestAnimationFrame) || w$2)(j$1)), t2.__H.__.some(function(n3) {
    n3.u && (n3.__H = n3.u), n3.u = void 0;
  })), u$1 = r$1 = null;
}, c$1.__c = function(n2, t2) {
  t2.some(function(n3) {
    try {
      n3.__h.some(z), n3.__h = n3.__h.filter(function(n4) {
        return !n4.__ || B(n4);
      });
    } catch (r2) {
      t2.some(function(n4) {
        n4.__h && (n4.__h = []);
      }), t2 = [], c$1.__e(r2, n3.__v);
    }
  }), l$2 && l$2(n2, t2);
}, c$1.unmount = function(n2) {
  m$1 && m$1(n2);
  var t2, r2 = n2.__c;
  r2 && r2.__H && (r2.__H.__.some(function(n3) {
    try {
      z(n3);
    } catch (n4) {
      t2 = n4;
    }
  }), r2.__H = void 0, t2 && c$1.__e(t2, r2.__v));
};
var k = "function" == typeof requestAnimationFrame;
function w$2(n2) {
  var t2, r2 = function() {
    clearTimeout(u4), k && cancelAnimationFrame(t2), setTimeout(n2);
  }, u4 = setTimeout(r2, 35);
  k && (t2 = requestAnimationFrame(r2));
}
function z(n2) {
  var t2 = r$1, u4 = n2.__c;
  "function" == typeof u4 && (n2.__c = void 0, u4()), r$1 = t2;
}
function B(n2) {
  var t2 = r$1;
  n2.__c = n2.__(), r$1 = t2;
}
function C(n2, t2) {
  return !n2 || n2.length !== t2.length || t2.some(function(t3, r2) {
    return t3 !== n2[r2];
  });
}
function D2(n2, t2) {
  return "function" == typeof t2 ? t2(n2) : t2;
}
var i = Symbol.for("preact-signals");
function t$1() {
  if (!(s > 1)) {
    var i2, t2 = false;
    !(function() {
      var i3 = c;
      c = void 0;
      while (void 0 !== i3) {
        if (i3.S.v === i3.v) i3.S.i = i3.i;
        i3 = i3.o;
      }
    })();
    while (void 0 !== h$1) {
      var n2 = h$1;
      h$1 = void 0;
      v++;
      while (void 0 !== n2) {
        var r2 = n2.u;
        n2.u = void 0;
        n2.f &= -3;
        if (!(8 & n2.f) && w$1(n2)) try {
          n2.c();
        } catch (n3) {
          if (!t2) {
            i2 = n3;
            t2 = true;
          }
        }
        n2 = r2;
      }
    }
    v = 0;
    s--;
    if (t2) throw i2;
  } else s--;
}
function n(i2) {
  if (s > 0) return i2();
  e = ++u3;
  s++;
  try {
    return i2();
  } finally {
    t$1();
  }
}
var r = void 0;
function o(i2) {
  var t2 = r;
  r = void 0;
  try {
    return i2();
  } finally {
    r = t2;
  }
}
var h$1 = void 0, s = 0, v = 0, u3 = 0, e = 0, c = void 0, d$1 = 0;
function a(i2) {
  if (void 0 !== r) {
    var t2 = i2.n;
    if (void 0 === t2 || t2.t !== r) {
      t2 = { i: 0, S: i2, p: r.s, n: void 0, t: r, e: void 0, x: void 0, r: t2 };
      if (void 0 !== r.s) r.s.n = t2;
      r.s = t2;
      i2.n = t2;
      if (32 & r.f) i2.S(t2);
      return t2;
    } else if (-1 === t2.i) {
      t2.i = 0;
      if (void 0 !== t2.n) {
        t2.n.p = t2.p;
        if (void 0 !== t2.p) t2.p.n = t2.n;
        t2.p = r.s;
        t2.n = void 0;
        r.s.n = t2;
        r.s = t2;
      }
      return t2;
    }
  }
}
function l$1(i2, t2) {
  this.v = i2;
  this.i = 0;
  this.n = void 0;
  this.t = void 0;
  this.l = 0;
  this.W = null == t2 ? void 0 : t2.watched;
  this.Z = null == t2 ? void 0 : t2.unwatched;
  this.name = null == t2 ? void 0 : t2.name;
}
l$1.prototype.brand = i;
l$1.prototype.h = function() {
  return true;
};
l$1.prototype.S = function(i2) {
  var t2 = this, n2 = this.t;
  if (n2 !== i2 && void 0 === i2.e) {
    i2.x = n2;
    this.t = i2;
    if (void 0 !== n2) n2.e = i2;
    else o(function() {
      var i3;
      null == (i3 = t2.W) || i3.call(t2);
    });
  }
};
l$1.prototype.U = function(i2) {
  var t2 = this;
  if (void 0 !== this.t) {
    var n2 = i2.e, r2 = i2.x;
    if (void 0 !== n2) {
      n2.x = r2;
      i2.e = void 0;
    }
    if (void 0 !== r2) {
      r2.e = n2;
      i2.x = void 0;
    }
    if (i2 === this.t) {
      this.t = r2;
      if (void 0 === r2) o(function() {
        var i3;
        null == (i3 = t2.Z) || i3.call(t2);
      });
    }
  }
};
l$1.prototype.subscribe = function(i2) {
  var t2 = this;
  return j(function() {
    var n2 = t2.value, o2 = r;
    r = void 0;
    try {
      i2(n2);
    } finally {
      r = o2;
    }
  }, { name: "sub" });
};
l$1.prototype.valueOf = function() {
  return this.value;
};
l$1.prototype.toString = function() {
  return this.value + "";
};
l$1.prototype.toJSON = function() {
  return this.value;
};
l$1.prototype.peek = function() {
  var i2 = r;
  r = void 0;
  try {
    return this.value;
  } finally {
    r = i2;
  }
};
Object.defineProperty(l$1.prototype, "value", { get: function() {
  var i2 = a(this);
  if (void 0 !== i2) i2.i = this.i;
  return this.v;
}, set: function(i2) {
  if (i2 !== this.v) {
    if (v > 100) throw new Error("Cycle detected");
    !(function(i3) {
      if (0 !== s && 0 === v) {
        if (i3.l !== e) {
          i3.l = e;
          c = { S: i3, v: i3.v, i: i3.i, o: c };
        }
      }
    })(this);
    this.v = i2;
    this.i++;
    d$1++;
    s++;
    try {
      for (var n2 = this.t; void 0 !== n2; n2 = n2.x) n2.t.N();
    } finally {
      t$1();
    }
  }
} });
function y$1(i2, t2) {
  return new l$1(i2, t2);
}
function w$1(i2) {
  for (var t2 = i2.s; void 0 !== t2; t2 = t2.n) if (t2.S.i !== t2.i || !t2.S.h() || t2.S.i !== t2.i) return true;
  return false;
}
function _$1(i2) {
  for (var t2 = i2.s; void 0 !== t2; t2 = t2.n) {
    var n2 = t2.S.n;
    if (void 0 !== n2) t2.r = n2;
    t2.S.n = t2;
    t2.i = -1;
    if (void 0 === t2.n) {
      i2.s = t2;
      break;
    }
  }
}
function b$1(i2) {
  var t2 = i2.s, n2 = void 0;
  while (void 0 !== t2) {
    var r2 = t2.p;
    if (-1 === t2.i) {
      t2.S.U(t2);
      if (void 0 !== r2) r2.n = t2.n;
      if (void 0 !== t2.n) t2.n.p = r2;
    } else n2 = t2;
    t2.S.n = t2.r;
    if (void 0 !== t2.r) t2.r = void 0;
    t2 = r2;
  }
  i2.s = n2;
}
function p$1(i2, t2) {
  l$1.call(this, void 0);
  this.x = i2;
  this.s = void 0;
  this.g = d$1 - 1;
  this.f = 4;
  this.W = null == t2 ? void 0 : t2.watched;
  this.Z = null == t2 ? void 0 : t2.unwatched;
  this.name = null == t2 ? void 0 : t2.name;
}
p$1.prototype = new l$1();
p$1.prototype.h = function() {
  this.f &= -3;
  if (1 & this.f) return false;
  if (32 == (36 & this.f)) return true;
  this.f &= -5;
  if (this.g === d$1) return true;
  this.g = d$1;
  this.f |= 1;
  if (this.i > 0 && !w$1(this)) {
    this.f &= -2;
    return true;
  }
  var i2 = r;
  try {
    _$1(this);
    r = this;
    var t2 = this.x();
    if (16 & this.f || this.v !== t2 || 0 === this.i) {
      this.v = t2;
      this.f &= -17;
      this.i++;
    }
  } catch (i3) {
    this.v = i3;
    this.f |= 16;
    this.i++;
  }
  r = i2;
  b$1(this);
  this.f &= -2;
  return true;
};
p$1.prototype.S = function(i2) {
  if (void 0 === this.t) {
    this.f |= 36;
    for (var t2 = this.s; void 0 !== t2; t2 = t2.n) t2.S.S(t2);
  }
  l$1.prototype.S.call(this, i2);
};
p$1.prototype.U = function(i2) {
  if (void 0 !== this.t) {
    l$1.prototype.U.call(this, i2);
    if (void 0 === this.t) {
      this.f &= -33;
      for (var t2 = this.s; void 0 !== t2; t2 = t2.n) t2.S.U(t2);
    }
  }
};
p$1.prototype.N = function() {
  if (!(2 & this.f)) {
    this.f |= 6;
    for (var i2 = this.t; void 0 !== i2; i2 = i2.x) i2.t.N();
  }
};
Object.defineProperty(p$1.prototype, "value", { get: function() {
  if (1 & this.f) throw new Error("Cycle detected");
  var i2 = a(this);
  this.h();
  if (void 0 !== i2) i2.i = this.i;
  if (16 & this.f) throw this.v;
  return this.v;
} });
function g$1(i2, t2) {
  return new p$1(i2, t2);
}
function S$1(i2) {
  var n2 = i2.m;
  i2.m = void 0;
  if ("function" == typeof n2) {
    s++;
    var o2 = r;
    r = void 0;
    try {
      n2();
    } catch (t2) {
      i2.f &= -2;
      i2.f |= 8;
      m(i2);
      throw t2;
    } finally {
      r = o2;
      t$1();
    }
  }
}
function m(i2) {
  for (var t2 = i2.s; void 0 !== t2; t2 = t2.n) t2.S.U(t2);
  i2.x = void 0;
  i2.s = void 0;
  S$1(i2);
}
function x$1(i2) {
  if (r !== this) throw new Error("Out-of-order effect");
  b$1(this);
  r = i2;
  this.f &= -2;
  if (8 & this.f) m(this);
  t$1();
}
function E(i2, t2) {
  this.x = i2;
  this.m = void 0;
  this.s = void 0;
  this.u = void 0;
  this.f = 32;
  this.name = null == t2 ? void 0 : t2.name;
}
E.prototype.c = function() {
  var i2 = this.S();
  try {
    if (8 & this.f) return;
    if (void 0 === this.x) return;
    var t2 = this.x();
    if ("function" == typeof t2) this.m = t2;
  } finally {
    i2();
  }
};
E.prototype.S = function() {
  if (1 & this.f) throw new Error("Cycle detected");
  this.f |= 1;
  this.f &= -9;
  S$1(this);
  _$1(this);
  s++;
  var i2 = r;
  r = this;
  return x$1.bind(this, i2);
};
E.prototype.N = function() {
  if (!(2 & this.f)) {
    this.f |= 2;
    this.u = h$1;
    h$1 = this;
  }
};
E.prototype.d = function() {
  this.f |= 8;
  if (!(1 & this.f)) m(this);
};
E.prototype.dispose = function() {
  this.d();
};
function j(i2, t2) {
  var n2 = new E(i2, t2);
  try {
    n2.c();
  } catch (i3) {
    n2.d();
    throw i3;
  }
  var r2 = n2.d.bind(n2);
  r2[Symbol.dispose] = r2;
  return r2;
}
var l, d, h, p = "undefined" != typeof window && !!window.__PREACT_SIGNALS_DEVTOOLS__, _ = [];
j(function() {
  l = this.N;
})();
function g(i2, r2) {
  l$3[i2] = r2.bind(null, l$3[i2] || function() {
  });
}
function b(i2) {
  if (h) {
    var n2 = h;
    h = void 0;
    n2();
  }
  h = i2 && i2.S();
}
function y2(i2) {
  var n2 = this, t2 = i2.data, e2 = useSignal(t2);
  e2.value = t2;
  var f2 = T(function() {
    var i3 = n2, t3 = n2.__v;
    while (t3 = t3.__) if (t3.__c) {
      t3.__c.__$f |= 4;
      break;
    }
    var o2 = g$1(function() {
      var i4 = e2.value.value;
      return 0 === i4 ? 0 : true === i4 ? "" : i4 || "";
    }), f3 = g$1(function() {
      return !Array.isArray(o2.value) && !t$3(o2.value);
    }), a3 = j(function() {
      this.N = F;
      if (f3.value) {
        var n3 = o2.value;
        if (i3.__v && i3.__v.__e && 3 === i3.__v.__e.nodeType) i3.__v.__e.data = n3;
      }
    }), v3 = n2.__$u.d;
    n2.__$u.d = function() {
      a3();
      v3.call(this);
    };
    return [f3, o2];
  }, []), a2 = f2[0], v2 = f2[1];
  return a2.value ? v2.peek() : v2.value;
}
y2.displayName = "ReactiveTextNode";
Object.defineProperties(l$1.prototype, { constructor: { configurable: true, value: void 0 }, type: { configurable: true, value: y2 }, props: { configurable: true, get: function() {
  var i2 = this;
  return { data: { get value() {
    return i2.value;
  } } };
} }, __b: { configurable: true, value: 1 } });
g("__b", function(i2, n2) {
  if ("string" == typeof n2.type) {
    var r2, t2 = n2.props;
    for (var o2 in t2) if ("children" !== o2) {
      var e2 = t2[o2];
      if (e2 instanceof l$1) {
        if (!r2) n2.__np = r2 = {};
        r2[o2] = e2;
        t2[o2] = e2.peek();
      }
    }
  }
  i2(n2);
});
g("__r", function(i2, n2) {
  i2(n2);
  if (n2.type !== S$2) {
    b();
    var r2, o2 = n2.__c;
    if (o2) {
      o2.__$f &= -2;
      if (void 0 === (r2 = o2.__$u)) o2.__$u = r2 = (function(i3, n3) {
        var r3;
        j(function() {
          r3 = this;
        }, { name: n3 });
        r3.c = i3;
        return r3;
      })(function() {
        var i3;
        if (p) null == (i3 = r2.y) || i3.call(r2);
        o2.__$f |= 1;
        o2.setState({});
      }, "function" == typeof n2.type ? n2.type.displayName || n2.type.name : "");
    }
    d = o2;
    b(r2);
  }
});
g("__e", function(i2, n2, r2, t2) {
  b();
  d = void 0;
  i2(n2, r2, t2);
});
g("diffed", function(i2, n2) {
  b();
  d = void 0;
  var r2;
  if ("string" == typeof n2.type && (r2 = n2.__e)) {
    var t2 = n2.__np, o2 = n2.props;
    if (t2) {
      var e2 = r2.U;
      if (e2) for (var f2 in e2) {
        var u4 = e2[f2];
        if (void 0 !== u4 && !(f2 in t2)) {
          u4.d();
          e2[f2] = void 0;
        }
      }
      else {
        e2 = {};
        r2.U = e2;
      }
      for (var a2 in t2) {
        var c2 = e2[a2], v2 = t2[a2];
        if (void 0 === c2) {
          c2 = w2(r2, a2, v2);
          e2[a2] = c2;
        } else c2.o(v2, o2);
      }
      for (var s2 in t2) o2[s2] = t2[s2];
    }
  }
  i2(n2);
});
function w2(i2, n2, r2, t2) {
  var o2 = n2 in i2 && void 0 === i2.ownerSVGElement, e2 = y$1(r2), f2 = r2.peek();
  return { o: function(i3, n3) {
    e2.value = i3;
    f2 = i3.peek();
  }, d: j(function() {
    this.N = F;
    var r3 = e2.value.value;
    if (f2 !== r3) {
      f2 = void 0;
      if (o2) i2[n2] = r3;
      else if (null != r3 && (false !== r3 || "-" === n2[4])) i2.setAttribute(n2, r3);
      else i2.removeAttribute(n2);
    } else f2 = void 0;
  }) };
}
g("unmount", function(i2, n2) {
  if ("string" == typeof n2.type) {
    var r2 = n2.__e;
    if (r2) {
      var t2 = r2.U;
      if (t2) {
        r2.U = void 0;
        for (var o2 in t2) {
          var e2 = t2[o2];
          if (e2) e2.d();
        }
      }
    }
    n2.__np = void 0;
  } else {
    var f2 = n2.__c;
    if (f2) {
      var u4 = f2.__$u;
      if (u4) {
        f2.__$u = void 0;
        u4.d();
      }
    }
  }
  i2(n2);
});
g("__h", function(i2, n2, r2, t2) {
  if (t2 < 3 || 9 === t2) n2.__$f |= 2;
  i2(n2, r2, t2);
});
C$2.prototype.shouldComponentUpdate = function(i2, n2) {
  if (this.__R) return true;
  var r2 = this.__$u, t2 = r2 && void 0 !== r2.s;
  for (var o2 in n2) return true;
  if (this.__f || "boolean" == typeof this.u && true === this.u) {
    var e2 = 2 & this.__$f;
    if (!(t2 || e2 || 4 & this.__$f)) return true;
    if (1 & this.__$f) return true;
  } else {
    if (!(t2 || 4 & this.__$f)) return true;
    if (3 & this.__$f) return true;
  }
  for (var f2 in i2) if ("__source" !== f2 && i2[f2] !== this.props[f2]) return true;
  for (var u4 in this.props) if (!(u4 in i2)) return true;
  return false;
};
function useSignal(i2, n2) {
  return T(function() {
    return y$1(i2, n2);
  }, []);
}
function useComputed(i2, n2) {
  var r2 = A(i2);
  r2.current = i2;
  d.__$f |= 4;
  return T(function() {
    return g$1(function() {
      return r2.current();
    }, n2);
  }, []);
}
var q = function(i2) {
  queueMicrotask(function() {
    queueMicrotask(i2);
  });
};
function x() {
  n(function() {
    var i2;
    while (i2 = _.shift()) l.call(i2);
  });
}
function F() {
  if (1 === _.push(this)) (l$3.requestAnimationFrame || q)(x);
}
const isString = (obj) => typeof obj === "string";
const defer = () => {
  let res;
  let rej;
  const promise = new Promise((resolve, reject) => {
    res = resolve;
    rej = reject;
  });
  promise.resolve = res;
  promise.reject = rej;
  return promise;
};
const makeString = (object) => {
  if (object == null) return "";
  return "" + object;
};
const copy = (a2, s2, t2) => {
  a2.forEach((m2) => {
    if (s2[m2]) t2[m2] = s2[m2];
  });
};
const lastOfPathSeparatorRegExp = /###/g;
const cleanKey = (key) => key && key.indexOf("###") > -1 ? key.replace(lastOfPathSeparatorRegExp, ".") : key;
const canNotTraverseDeeper = (object) => !object || isString(object);
const getLastOfPath = (object, path, Empty) => {
  const stack = !isString(path) ? path : path.split(".");
  let stackIndex = 0;
  while (stackIndex < stack.length - 1) {
    if (canNotTraverseDeeper(object)) return {};
    const key = cleanKey(stack[stackIndex]);
    if (!object[key] && Empty) object[key] = new Empty();
    if (Object.prototype.hasOwnProperty.call(object, key)) {
      object = object[key];
    } else {
      object = {};
    }
    ++stackIndex;
  }
  if (canNotTraverseDeeper(object)) return {};
  return {
    obj: object,
    k: cleanKey(stack[stackIndex])
  };
};
const setPath = (object, path, newValue) => {
  const {
    obj,
    k: k2
  } = getLastOfPath(object, path, Object);
  if (obj !== void 0 || path.length === 1) {
    obj[k2] = newValue;
    return;
  }
  let e2 = path[path.length - 1];
  let p2 = path.slice(0, path.length - 1);
  let last = getLastOfPath(object, p2, Object);
  while (last.obj === void 0 && p2.length) {
    e2 = `${p2[p2.length - 1]}.${e2}`;
    p2 = p2.slice(0, p2.length - 1);
    last = getLastOfPath(object, p2, Object);
    if ((last == null ? void 0 : last.obj) && typeof last.obj[`${last.k}.${e2}`] !== "undefined") {
      last.obj = void 0;
    }
  }
  last.obj[`${last.k}.${e2}`] = newValue;
};
const pushPath = (object, path, newValue, concat) => {
  const {
    obj,
    k: k2
  } = getLastOfPath(object, path, Object);
  obj[k2] = obj[k2] || [];
  obj[k2].push(newValue);
};
const getPath = (object, path) => {
  const {
    obj,
    k: k2
  } = getLastOfPath(object, path);
  if (!obj) return void 0;
  if (!Object.prototype.hasOwnProperty.call(obj, k2)) return void 0;
  return obj[k2];
};
const getPathWithDefaults = (data, defaultData, key) => {
  const value = getPath(data, key);
  if (value !== void 0) {
    return value;
  }
  return getPath(defaultData, key);
};
const deepExtend = (target, source, overwrite) => {
  for (const prop in source) {
    if (prop !== "__proto__" && prop !== "constructor") {
      if (prop in target) {
        if (isString(target[prop]) || target[prop] instanceof String || isString(source[prop]) || source[prop] instanceof String) {
          if (overwrite) target[prop] = source[prop];
        } else {
          deepExtend(target[prop], source[prop], overwrite);
        }
      } else {
        target[prop] = source[prop];
      }
    }
  }
  return target;
};
const regexEscape = (str) => str.replace(/[\-\[\]\/\{\}\(\)\*\+\?\.\\\^\$\|]/g, "\\$&");
var _entityMap = {
  "&": "&amp;",
  "<": "&lt;",
  ">": "&gt;",
  '"': "&quot;",
  "'": "&#39;",
  "/": "&#x2F;"
};
const escape = (data) => {
  if (isString(data)) {
    return data.replace(/[&<>"'\/]/g, (s2) => _entityMap[s2]);
  }
  return data;
};
class RegExpCache {
  constructor(capacity) {
    this.capacity = capacity;
    this.regExpMap = /* @__PURE__ */ new Map();
    this.regExpQueue = [];
  }
  getRegExp(pattern) {
    const regExpFromCache = this.regExpMap.get(pattern);
    if (regExpFromCache !== void 0) {
      return regExpFromCache;
    }
    const regExpNew = new RegExp(pattern);
    if (this.regExpQueue.length === this.capacity) {
      this.regExpMap.delete(this.regExpQueue.shift());
    }
    this.regExpMap.set(pattern, regExpNew);
    this.regExpQueue.push(pattern);
    return regExpNew;
  }
}
const chars = [" ", ",", "?", "!", ";"];
const looksLikeObjectPathRegExpCache = new RegExpCache(20);
const looksLikeObjectPath = (key, nsSeparator, keySeparator) => {
  nsSeparator = nsSeparator || "";
  keySeparator = keySeparator || "";
  const possibleChars = chars.filter((c2) => nsSeparator.indexOf(c2) < 0 && keySeparator.indexOf(c2) < 0);
  if (possibleChars.length === 0) return true;
  const r2 = looksLikeObjectPathRegExpCache.getRegExp(`(${possibleChars.map((c2) => c2 === "?" ? "\\?" : c2).join("|")})`);
  let matched = !r2.test(key);
  if (!matched) {
    const ki = key.indexOf(keySeparator);
    if (ki > 0 && !r2.test(key.substring(0, ki))) {
      matched = true;
    }
  }
  return matched;
};
const deepFind = function(obj, path) {
  let keySeparator = arguments.length > 2 && arguments[2] !== void 0 ? arguments[2] : ".";
  if (!obj) return void 0;
  if (obj[path]) {
    if (!Object.prototype.hasOwnProperty.call(obj, path)) return void 0;
    return obj[path];
  }
  const tokens = path.split(keySeparator);
  let current = obj;
  for (let i2 = 0; i2 < tokens.length; ) {
    if (!current || typeof current !== "object") {
      return void 0;
    }
    let next;
    let nextPath = "";
    for (let j2 = i2; j2 < tokens.length; ++j2) {
      if (j2 !== i2) {
        nextPath += keySeparator;
      }
      nextPath += tokens[j2];
      next = current[nextPath];
      if (next !== void 0) {
        if (["string", "number", "boolean"].indexOf(typeof next) > -1 && j2 < tokens.length - 1) {
          continue;
        }
        i2 += j2 - i2 + 1;
        break;
      }
    }
    current = next;
  }
  return current;
};
const getCleanedCode = (code) => code == null ? void 0 : code.replace("_", "-");
const consoleLogger = {
  type: "logger",
  log(args) {
    this.output("log", args);
  },
  warn(args) {
    this.output("warn", args);
  },
  error(args) {
    this.output("error", args);
  },
  output(type, args) {
    var _a2, _b;
    (_b = (_a2 = console == null ? void 0 : console[type]) == null ? void 0 : _a2.apply) == null ? void 0 : _b.call(_a2, console, args);
  }
};
class Logger {
  constructor(concreteLogger) {
    let options = arguments.length > 1 && arguments[1] !== void 0 ? arguments[1] : {};
    this.init(concreteLogger, options);
  }
  init(concreteLogger) {
    let options = arguments.length > 1 && arguments[1] !== void 0 ? arguments[1] : {};
    this.prefix = options.prefix || "i18next:";
    this.logger = concreteLogger || consoleLogger;
    this.options = options;
    this.debug = options.debug;
  }
  log() {
    for (var _len = arguments.length, args = new Array(_len), _key = 0; _key < _len; _key++) {
      args[_key] = arguments[_key];
    }
    return this.forward(args, "log", "", true);
  }
  warn() {
    for (var _len2 = arguments.length, args = new Array(_len2), _key2 = 0; _key2 < _len2; _key2++) {
      args[_key2] = arguments[_key2];
    }
    return this.forward(args, "warn", "", true);
  }
  error() {
    for (var _len3 = arguments.length, args = new Array(_len3), _key3 = 0; _key3 < _len3; _key3++) {
      args[_key3] = arguments[_key3];
    }
    return this.forward(args, "error", "");
  }
  deprecate() {
    for (var _len4 = arguments.length, args = new Array(_len4), _key4 = 0; _key4 < _len4; _key4++) {
      args[_key4] = arguments[_key4];
    }
    return this.forward(args, "warn", "WARNING DEPRECATED: ", true);
  }
  forward(args, lvl, prefix, debugOnly) {
    if (debugOnly && !this.debug) return null;
    if (isString(args[0])) args[0] = `${prefix}${this.prefix} ${args[0]}`;
    return this.logger[lvl](args);
  }
  create(moduleName) {
    return new Logger(this.logger, {
      ...{
        prefix: `${this.prefix}:${moduleName}:`
      },
      ...this.options
    });
  }
  clone(options) {
    options = options || this.options;
    options.prefix = options.prefix || this.prefix;
    return new Logger(this.logger, options);
  }
}
var baseLogger = new Logger();
class EventEmitter {
  constructor() {
    this.observers = {};
  }
  on(events, listener) {
    events.split(" ").forEach((event) => {
      if (!this.observers[event]) this.observers[event] = /* @__PURE__ */ new Map();
      const numListeners = this.observers[event].get(listener) || 0;
      this.observers[event].set(listener, numListeners + 1);
    });
    return this;
  }
  off(event, listener) {
    if (!this.observers[event]) return;
    if (!listener) {
      delete this.observers[event];
      return;
    }
    this.observers[event].delete(listener);
  }
  emit(event) {
    for (var _len = arguments.length, args = new Array(_len > 1 ? _len - 1 : 0), _key = 1; _key < _len; _key++) {
      args[_key - 1] = arguments[_key];
    }
    if (this.observers[event]) {
      const cloned = Array.from(this.observers[event].entries());
      cloned.forEach((_ref) => {
        let [observer, numTimesAdded] = _ref;
        for (let i2 = 0; i2 < numTimesAdded; i2++) {
          observer(...args);
        }
      });
    }
    if (this.observers["*"]) {
      const cloned = Array.from(this.observers["*"].entries());
      cloned.forEach((_ref2) => {
        let [observer, numTimesAdded] = _ref2;
        for (let i2 = 0; i2 < numTimesAdded; i2++) {
          observer.apply(observer, [event, ...args]);
        }
      });
    }
  }
}
class ResourceStore extends EventEmitter {
  constructor(data) {
    let options = arguments.length > 1 && arguments[1] !== void 0 ? arguments[1] : {
      ns: ["translation"],
      defaultNS: "translation"
    };
    super();
    this.data = data || {};
    this.options = options;
    if (this.options.keySeparator === void 0) {
      this.options.keySeparator = ".";
    }
    if (this.options.ignoreJSONStructure === void 0) {
      this.options.ignoreJSONStructure = true;
    }
  }
  addNamespaces(ns) {
    if (this.options.ns.indexOf(ns) < 0) {
      this.options.ns.push(ns);
    }
  }
  removeNamespaces(ns) {
    const index = this.options.ns.indexOf(ns);
    if (index > -1) {
      this.options.ns.splice(index, 1);
    }
  }
  getResource(lng, ns, key) {
    var _a2, _b;
    let options = arguments.length > 3 && arguments[3] !== void 0 ? arguments[3] : {};
    const keySeparator = options.keySeparator !== void 0 ? options.keySeparator : this.options.keySeparator;
    const ignoreJSONStructure = options.ignoreJSONStructure !== void 0 ? options.ignoreJSONStructure : this.options.ignoreJSONStructure;
    let path;
    if (lng.indexOf(".") > -1) {
      path = lng.split(".");
    } else {
      path = [lng, ns];
      if (key) {
        if (Array.isArray(key)) {
          path.push(...key);
        } else if (isString(key) && keySeparator) {
          path.push(...key.split(keySeparator));
        } else {
          path.push(key);
        }
      }
    }
    const result = getPath(this.data, path);
    if (!result && !ns && !key && lng.indexOf(".") > -1) {
      lng = path[0];
      ns = path[1];
      key = path.slice(2).join(".");
    }
    if (result || !ignoreJSONStructure || !isString(key)) return result;
    return deepFind((_b = (_a2 = this.data) == null ? void 0 : _a2[lng]) == null ? void 0 : _b[ns], key, keySeparator);
  }
  addResource(lng, ns, key, value) {
    let options = arguments.length > 4 && arguments[4] !== void 0 ? arguments[4] : {
      silent: false
    };
    const keySeparator = options.keySeparator !== void 0 ? options.keySeparator : this.options.keySeparator;
    let path = [lng, ns];
    if (key) path = path.concat(keySeparator ? key.split(keySeparator) : key);
    if (lng.indexOf(".") > -1) {
      path = lng.split(".");
      value = ns;
      ns = path[1];
    }
    this.addNamespaces(ns);
    setPath(this.data, path, value);
    if (!options.silent) this.emit("added", lng, ns, key, value);
  }
  addResources(lng, ns, resources) {
    let options = arguments.length > 3 && arguments[3] !== void 0 ? arguments[3] : {
      silent: false
    };
    for (const m2 in resources) {
      if (isString(resources[m2]) || Array.isArray(resources[m2])) this.addResource(lng, ns, m2, resources[m2], {
        silent: true
      });
    }
    if (!options.silent) this.emit("added", lng, ns, resources);
  }
  addResourceBundle(lng, ns, resources, deep, overwrite) {
    let options = arguments.length > 5 && arguments[5] !== void 0 ? arguments[5] : {
      silent: false,
      skipCopy: false
    };
    let path = [lng, ns];
    if (lng.indexOf(".") > -1) {
      path = lng.split(".");
      deep = resources;
      resources = ns;
      ns = path[1];
    }
    this.addNamespaces(ns);
    let pack = getPath(this.data, path) || {};
    if (!options.skipCopy) resources = JSON.parse(JSON.stringify(resources));
    if (deep) {
      deepExtend(pack, resources, overwrite);
    } else {
      pack = {
        ...pack,
        ...resources
      };
    }
    setPath(this.data, path, pack);
    if (!options.silent) this.emit("added", lng, ns, resources);
  }
  removeResourceBundle(lng, ns) {
    if (this.hasResourceBundle(lng, ns)) {
      delete this.data[lng][ns];
    }
    this.removeNamespaces(ns);
    this.emit("removed", lng, ns);
  }
  hasResourceBundle(lng, ns) {
    return this.getResource(lng, ns) !== void 0;
  }
  getResourceBundle(lng, ns) {
    if (!ns) ns = this.options.defaultNS;
    return this.getResource(lng, ns);
  }
  getDataByLanguage(lng) {
    return this.data[lng];
  }
  hasLanguageSomeTranslations(lng) {
    const data = this.getDataByLanguage(lng);
    const n2 = data && Object.keys(data) || [];
    return !!n2.find((v2) => data[v2] && Object.keys(data[v2]).length > 0);
  }
  toJSON() {
    return this.data;
  }
}
var postProcessor = {
  processors: {},
  addPostProcessor(module) {
    this.processors[module.name] = module;
  },
  handle(processors, value, key, options, translator) {
    processors.forEach((processor) => {
      var _a2;
      value = ((_a2 = this.processors[processor]) == null ? void 0 : _a2.process(value, key, options, translator)) ?? value;
    });
    return value;
  }
};
const checkedLoadedFor = {};
const shouldHandleAsObject = (res) => !isString(res) && typeof res !== "boolean" && typeof res !== "number";
class Translator extends EventEmitter {
  constructor(services) {
    let options = arguments.length > 1 && arguments[1] !== void 0 ? arguments[1] : {};
    super();
    copy(["resourceStore", "languageUtils", "pluralResolver", "interpolator", "backendConnector", "i18nFormat", "utils"], services, this);
    this.options = options;
    if (this.options.keySeparator === void 0) {
      this.options.keySeparator = ".";
    }
    this.logger = baseLogger.create("translator");
  }
  changeLanguage(lng) {
    if (lng) this.language = lng;
  }
  exists(key) {
    let options = arguments.length > 1 && arguments[1] !== void 0 ? arguments[1] : {
      interpolation: {}
    };
    if (key == null) {
      return false;
    }
    const resolved = this.resolve(key, options);
    return (resolved == null ? void 0 : resolved.res) !== void 0;
  }
  extractFromKey(key, options) {
    let nsSeparator = options.nsSeparator !== void 0 ? options.nsSeparator : this.options.nsSeparator;
    if (nsSeparator === void 0) nsSeparator = ":";
    const keySeparator = options.keySeparator !== void 0 ? options.keySeparator : this.options.keySeparator;
    let namespaces2 = options.ns || this.options.defaultNS || [];
    const wouldCheckForNsInKey = nsSeparator && key.indexOf(nsSeparator) > -1;
    const seemsNaturalLanguage = !this.options.userDefinedKeySeparator && !options.keySeparator && !this.options.userDefinedNsSeparator && !options.nsSeparator && !looksLikeObjectPath(key, nsSeparator, keySeparator);
    if (wouldCheckForNsInKey && !seemsNaturalLanguage) {
      const m2 = key.match(this.interpolator.nestingRegexp);
      if (m2 && m2.length > 0) {
        return {
          key,
          namespaces: isString(namespaces2) ? [namespaces2] : namespaces2
        };
      }
      const parts = key.split(nsSeparator);
      if (nsSeparator !== keySeparator || nsSeparator === keySeparator && this.options.ns.indexOf(parts[0]) > -1) namespaces2 = parts.shift();
      key = parts.join(keySeparator);
    }
    return {
      key,
      namespaces: isString(namespaces2) ? [namespaces2] : namespaces2
    };
  }
  translate(keys, options, lastKey) {
    if (typeof options !== "object" && this.options.overloadTranslationOptionHandler) {
      options = this.options.overloadTranslationOptionHandler(arguments);
    }
    if (typeof options === "object") options = {
      ...options
    };
    if (!options) options = {};
    if (keys == null) return "";
    if (!Array.isArray(keys)) keys = [String(keys)];
    const returnDetails = options.returnDetails !== void 0 ? options.returnDetails : this.options.returnDetails;
    const keySeparator = options.keySeparator !== void 0 ? options.keySeparator : this.options.keySeparator;
    const {
      key,
      namespaces: namespaces2
    } = this.extractFromKey(keys[keys.length - 1], options);
    const namespace = namespaces2[namespaces2.length - 1];
    const lng = options.lng || this.language;
    const appendNamespaceToCIMode = options.appendNamespaceToCIMode || this.options.appendNamespaceToCIMode;
    if ((lng == null ? void 0 : lng.toLowerCase()) === "cimode") {
      if (appendNamespaceToCIMode) {
        const nsSeparator = options.nsSeparator || this.options.nsSeparator;
        if (returnDetails) {
          return {
            res: `${namespace}${nsSeparator}${key}`,
            usedKey: key,
            exactUsedKey: key,
            usedLng: lng,
            usedNS: namespace,
            usedParams: this.getUsedParamsDetails(options)
          };
        }
        return `${namespace}${nsSeparator}${key}`;
      }
      if (returnDetails) {
        return {
          res: key,
          usedKey: key,
          exactUsedKey: key,
          usedLng: lng,
          usedNS: namespace,
          usedParams: this.getUsedParamsDetails(options)
        };
      }
      return key;
    }
    const resolved = this.resolve(keys, options);
    let res = resolved == null ? void 0 : resolved.res;
    const resUsedKey = (resolved == null ? void 0 : resolved.usedKey) || key;
    const resExactUsedKey = (resolved == null ? void 0 : resolved.exactUsedKey) || key;
    const noObject = ["[object Number]", "[object Function]", "[object RegExp]"];
    const joinArrays = options.joinArrays !== void 0 ? options.joinArrays : this.options.joinArrays;
    const handleAsObjectInI18nFormat = !this.i18nFormat || this.i18nFormat.handleAsObject;
    const needsPluralHandling = options.count !== void 0 && !isString(options.count);
    const hasDefaultValue = Translator.hasDefaultValue(options);
    const defaultValueSuffix = needsPluralHandling ? this.pluralResolver.getSuffix(lng, options.count, options) : "";
    const defaultValueSuffixOrdinalFallback = options.ordinal && needsPluralHandling ? this.pluralResolver.getSuffix(lng, options.count, {
      ordinal: false
    }) : "";
    const needsZeroSuffixLookup = needsPluralHandling && !options.ordinal && options.count === 0;
    const defaultValue = needsZeroSuffixLookup && options[`defaultValue${this.options.pluralSeparator}zero`] || options[`defaultValue${defaultValueSuffix}`] || options[`defaultValue${defaultValueSuffixOrdinalFallback}`] || options.defaultValue;
    let resForObjHndl = res;
    if (handleAsObjectInI18nFormat && !res && hasDefaultValue) {
      resForObjHndl = defaultValue;
    }
    const handleAsObject = shouldHandleAsObject(resForObjHndl);
    const resType = Object.prototype.toString.apply(resForObjHndl);
    if (handleAsObjectInI18nFormat && resForObjHndl && handleAsObject && noObject.indexOf(resType) < 0 && !(isString(joinArrays) && Array.isArray(resForObjHndl))) {
      if (!options.returnObjects && !this.options.returnObjects) {
        if (!this.options.returnedObjectHandler) {
          this.logger.warn("accessing an object - but returnObjects options is not enabled!");
        }
        const r2 = this.options.returnedObjectHandler ? this.options.returnedObjectHandler(resUsedKey, resForObjHndl, {
          ...options,
          ns: namespaces2
        }) : `key '${key} (${this.language})' returned an object instead of string.`;
        if (returnDetails) {
          resolved.res = r2;
          resolved.usedParams = this.getUsedParamsDetails(options);
          return resolved;
        }
        return r2;
      }
      if (keySeparator) {
        const resTypeIsArray = Array.isArray(resForObjHndl);
        const copy2 = resTypeIsArray ? [] : {};
        const newKeyToUse = resTypeIsArray ? resExactUsedKey : resUsedKey;
        for (const m2 in resForObjHndl) {
          if (Object.prototype.hasOwnProperty.call(resForObjHndl, m2)) {
            const deepKey = `${newKeyToUse}${keySeparator}${m2}`;
            if (hasDefaultValue && !res) {
              copy2[m2] = this.translate(deepKey, {
                ...options,
                defaultValue: shouldHandleAsObject(defaultValue) ? defaultValue[m2] : void 0,
                ...{
                  joinArrays: false,
                  ns: namespaces2
                }
              });
            } else {
              copy2[m2] = this.translate(deepKey, {
                ...options,
                ...{
                  joinArrays: false,
                  ns: namespaces2
                }
              });
            }
            if (copy2[m2] === deepKey) copy2[m2] = resForObjHndl[m2];
          }
        }
        res = copy2;
      }
    } else if (handleAsObjectInI18nFormat && isString(joinArrays) && Array.isArray(res)) {
      res = res.join(joinArrays);
      if (res) res = this.extendTranslation(res, keys, options, lastKey);
    } else {
      let usedDefault = false;
      let usedKey = false;
      if (!this.isValidLookup(res) && hasDefaultValue) {
        usedDefault = true;
        res = defaultValue;
      }
      if (!this.isValidLookup(res)) {
        usedKey = true;
        res = key;
      }
      const missingKeyNoValueFallbackToKey = options.missingKeyNoValueFallbackToKey || this.options.missingKeyNoValueFallbackToKey;
      const resForMissing = missingKeyNoValueFallbackToKey && usedKey ? void 0 : res;
      const updateMissing = hasDefaultValue && defaultValue !== res && this.options.updateMissing;
      if (usedKey || usedDefault || updateMissing) {
        this.logger.log(updateMissing ? "updateKey" : "missingKey", lng, namespace, key, updateMissing ? defaultValue : res);
        if (keySeparator) {
          const fk = this.resolve(key, {
            ...options,
            keySeparator: false
          });
          if (fk && fk.res) this.logger.warn("Seems the loaded translations were in flat JSON format instead of nested. Either set keySeparator: false on init or make sure your translations are published in nested format.");
        }
        let lngs = [];
        const fallbackLngs = this.languageUtils.getFallbackCodes(this.options.fallbackLng, options.lng || this.language);
        if (this.options.saveMissingTo === "fallback" && fallbackLngs && fallbackLngs[0]) {
          for (let i2 = 0; i2 < fallbackLngs.length; i2++) {
            lngs.push(fallbackLngs[i2]);
          }
        } else if (this.options.saveMissingTo === "all") {
          lngs = this.languageUtils.toResolveHierarchy(options.lng || this.language);
        } else {
          lngs.push(options.lng || this.language);
        }
        const send = (l2, k2, specificDefaultValue) => {
          var _a2;
          const defaultForMissing = hasDefaultValue && specificDefaultValue !== res ? specificDefaultValue : resForMissing;
          if (this.options.missingKeyHandler) {
            this.options.missingKeyHandler(l2, namespace, k2, defaultForMissing, updateMissing, options);
          } else if ((_a2 = this.backendConnector) == null ? void 0 : _a2.saveMissing) {
            this.backendConnector.saveMissing(l2, namespace, k2, defaultForMissing, updateMissing, options);
          }
          this.emit("missingKey", l2, namespace, k2, res);
        };
        if (this.options.saveMissing) {
          if (this.options.saveMissingPlurals && needsPluralHandling) {
            lngs.forEach((language) => {
              const suffixes = this.pluralResolver.getSuffixes(language, options);
              if (needsZeroSuffixLookup && options[`defaultValue${this.options.pluralSeparator}zero`] && suffixes.indexOf(`${this.options.pluralSeparator}zero`) < 0) {
                suffixes.push(`${this.options.pluralSeparator}zero`);
              }
              suffixes.forEach((suffix) => {
                send([language], key + suffix, options[`defaultValue${suffix}`] || defaultValue);
              });
            });
          } else {
            send(lngs, key, defaultValue);
          }
        }
      }
      res = this.extendTranslation(res, keys, options, resolved, lastKey);
      if (usedKey && res === key && this.options.appendNamespaceToMissingKey) res = `${namespace}:${key}`;
      if ((usedKey || usedDefault) && this.options.parseMissingKeyHandler) {
        res = this.options.parseMissingKeyHandler(this.options.appendNamespaceToMissingKey ? `${namespace}:${key}` : key, usedDefault ? res : void 0);
      }
    }
    if (returnDetails) {
      resolved.res = res;
      resolved.usedParams = this.getUsedParamsDetails(options);
      return resolved;
    }
    return res;
  }
  extendTranslation(res, key, options, resolved, lastKey) {
    var _a2, _b;
    var _this = this;
    if ((_a2 = this.i18nFormat) == null ? void 0 : _a2.parse) {
      res = this.i18nFormat.parse(res, {
        ...this.options.interpolation.defaultVariables,
        ...options
      }, options.lng || this.language || resolved.usedLng, resolved.usedNS, resolved.usedKey, {
        resolved
      });
    } else if (!options.skipInterpolation) {
      if (options.interpolation) this.interpolator.init({
        ...options,
        ...{
          interpolation: {
            ...this.options.interpolation,
            ...options.interpolation
          }
        }
      });
      const skipOnVariables = isString(res) && (((_b = options == null ? void 0 : options.interpolation) == null ? void 0 : _b.skipOnVariables) !== void 0 ? options.interpolation.skipOnVariables : this.options.interpolation.skipOnVariables);
      let nestBef;
      if (skipOnVariables) {
        const nb = res.match(this.interpolator.nestingRegexp);
        nestBef = nb && nb.length;
      }
      let data = options.replace && !isString(options.replace) ? options.replace : options;
      if (this.options.interpolation.defaultVariables) data = {
        ...this.options.interpolation.defaultVariables,
        ...data
      };
      res = this.interpolator.interpolate(res, data, options.lng || this.language || resolved.usedLng, options);
      if (skipOnVariables) {
        const na = res.match(this.interpolator.nestingRegexp);
        const nestAft = na && na.length;
        if (nestBef < nestAft) options.nest = false;
      }
      if (!options.lng && resolved && resolved.res) options.lng = this.language || resolved.usedLng;
      if (options.nest !== false) res = this.interpolator.nest(res, function() {
        for (var _len = arguments.length, args = new Array(_len), _key = 0; _key < _len; _key++) {
          args[_key] = arguments[_key];
        }
        if ((lastKey == null ? void 0 : lastKey[0]) === args[0] && !options.context) {
          _this.logger.warn(`It seems you are nesting recursively key: ${args[0]} in key: ${key[0]}`);
          return null;
        }
        return _this.translate(...args, key);
      }, options);
      if (options.interpolation) this.interpolator.reset();
    }
    const postProcess = options.postProcess || this.options.postProcess;
    const postProcessorNames = isString(postProcess) ? [postProcess] : postProcess;
    if (res != null && (postProcessorNames == null ? void 0 : postProcessorNames.length) && options.applyPostProcessor !== false) {
      res = postProcessor.handle(postProcessorNames, res, key, this.options && this.options.postProcessPassResolved ? {
        i18nResolved: {
          ...resolved,
          usedParams: this.getUsedParamsDetails(options)
        },
        ...options
      } : options, this);
    }
    return res;
  }
  resolve(keys) {
    let options = arguments.length > 1 && arguments[1] !== void 0 ? arguments[1] : {};
    let found;
    let usedKey;
    let exactUsedKey;
    let usedLng;
    let usedNS;
    if (isString(keys)) keys = [keys];
    keys.forEach((k2) => {
      if (this.isValidLookup(found)) return;
      const extracted = this.extractFromKey(k2, options);
      const key = extracted.key;
      usedKey = key;
      let namespaces2 = extracted.namespaces;
      if (this.options.fallbackNS) namespaces2 = namespaces2.concat(this.options.fallbackNS);
      const needsPluralHandling = options.count !== void 0 && !isString(options.count);
      const needsZeroSuffixLookup = needsPluralHandling && !options.ordinal && options.count === 0;
      const needsContextHandling = options.context !== void 0 && (isString(options.context) || typeof options.context === "number") && options.context !== "";
      const codes = options.lngs ? options.lngs : this.languageUtils.toResolveHierarchy(options.lng || this.language, options.fallbackLng);
      namespaces2.forEach((ns) => {
        var _a2, _b;
        if (this.isValidLookup(found)) return;
        usedNS = ns;
        if (!checkedLoadedFor[`${codes[0]}-${ns}`] && ((_a2 = this.utils) == null ? void 0 : _a2.hasLoadedNamespace) && !((_b = this.utils) == null ? void 0 : _b.hasLoadedNamespace(usedNS))) {
          checkedLoadedFor[`${codes[0]}-${ns}`] = true;
          this.logger.warn(`key "${usedKey}" for languages "${codes.join(", ")}" won't get resolved as namespace "${usedNS}" was not yet loaded`, "This means something IS WRONG in your setup. You access the t function before i18next.init / i18next.loadNamespace / i18next.changeLanguage was done. Wait for the callback or Promise to resolve before accessing it!!!");
        }
        codes.forEach((code) => {
          var _a3;
          if (this.isValidLookup(found)) return;
          usedLng = code;
          const finalKeys = [key];
          if ((_a3 = this.i18nFormat) == null ? void 0 : _a3.addLookupKeys) {
            this.i18nFormat.addLookupKeys(finalKeys, key, code, ns, options);
          } else {
            let pluralSuffix;
            if (needsPluralHandling) pluralSuffix = this.pluralResolver.getSuffix(code, options.count, options);
            const zeroSuffix = `${this.options.pluralSeparator}zero`;
            const ordinalPrefix = `${this.options.pluralSeparator}ordinal${this.options.pluralSeparator}`;
            if (needsPluralHandling) {
              finalKeys.push(key + pluralSuffix);
              if (options.ordinal && pluralSuffix.indexOf(ordinalPrefix) === 0) {
                finalKeys.push(key + pluralSuffix.replace(ordinalPrefix, this.options.pluralSeparator));
              }
              if (needsZeroSuffixLookup) {
                finalKeys.push(key + zeroSuffix);
              }
            }
            if (needsContextHandling) {
              const contextKey = `${key}${this.options.contextSeparator}${options.context}`;
              finalKeys.push(contextKey);
              if (needsPluralHandling) {
                finalKeys.push(contextKey + pluralSuffix);
                if (options.ordinal && pluralSuffix.indexOf(ordinalPrefix) === 0) {
                  finalKeys.push(contextKey + pluralSuffix.replace(ordinalPrefix, this.options.pluralSeparator));
                }
                if (needsZeroSuffixLookup) {
                  finalKeys.push(contextKey + zeroSuffix);
                }
              }
            }
          }
          let possibleKey;
          while (possibleKey = finalKeys.pop()) {
            if (!this.isValidLookup(found)) {
              exactUsedKey = possibleKey;
              found = this.getResource(code, ns, possibleKey, options);
            }
          }
        });
      });
    });
    return {
      res: found,
      usedKey,
      exactUsedKey,
      usedLng,
      usedNS
    };
  }
  isValidLookup(res) {
    return res !== void 0 && !(!this.options.returnNull && res === null) && !(!this.options.returnEmptyString && res === "");
  }
  getResource(code, ns, key) {
    var _a2;
    let options = arguments.length > 3 && arguments[3] !== void 0 ? arguments[3] : {};
    if ((_a2 = this.i18nFormat) == null ? void 0 : _a2.getResource) return this.i18nFormat.getResource(code, ns, key, options);
    return this.resourceStore.getResource(code, ns, key, options);
  }
  getUsedParamsDetails() {
    let options = arguments.length > 0 && arguments[0] !== void 0 ? arguments[0] : {};
    const optionsKeys = ["defaultValue", "ordinal", "context", "replace", "lng", "lngs", "fallbackLng", "ns", "keySeparator", "nsSeparator", "returnObjects", "returnDetails", "joinArrays", "postProcess", "interpolation"];
    const useOptionsReplaceForData = options.replace && !isString(options.replace);
    let data = useOptionsReplaceForData ? options.replace : options;
    if (useOptionsReplaceForData && typeof options.count !== "undefined") {
      data.count = options.count;
    }
    if (this.options.interpolation.defaultVariables) {
      data = {
        ...this.options.interpolation.defaultVariables,
        ...data
      };
    }
    if (!useOptionsReplaceForData) {
      data = {
        ...data
      };
      for (const key of optionsKeys) {
        delete data[key];
      }
    }
    return data;
  }
  static hasDefaultValue(options) {
    const prefix = "defaultValue";
    for (const option in options) {
      if (Object.prototype.hasOwnProperty.call(options, option) && prefix === option.substring(0, prefix.length) && void 0 !== options[option]) {
        return true;
      }
    }
    return false;
  }
}
class LanguageUtil {
  constructor(options) {
    this.options = options;
    this.supportedLngs = this.options.supportedLngs || false;
    this.logger = baseLogger.create("languageUtils");
  }
  getScriptPartFromCode(code) {
    code = getCleanedCode(code);
    if (!code || code.indexOf("-") < 0) return null;
    const p2 = code.split("-");
    if (p2.length === 2) return null;
    p2.pop();
    if (p2[p2.length - 1].toLowerCase() === "x") return null;
    return this.formatLanguageCode(p2.join("-"));
  }
  getLanguagePartFromCode(code) {
    code = getCleanedCode(code);
    if (!code || code.indexOf("-") < 0) return code;
    const p2 = code.split("-");
    return this.formatLanguageCode(p2[0]);
  }
  formatLanguageCode(code) {
    if (isString(code) && code.indexOf("-") > -1) {
      let formattedCode;
      try {
        formattedCode = Intl.getCanonicalLocales(code)[0];
      } catch (e2) {
      }
      if (formattedCode && this.options.lowerCaseLng) {
        formattedCode = formattedCode.toLowerCase();
      }
      if (formattedCode) return formattedCode;
      if (this.options.lowerCaseLng) {
        return code.toLowerCase();
      }
      return code;
    }
    return this.options.cleanCode || this.options.lowerCaseLng ? code.toLowerCase() : code;
  }
  isSupportedCode(code) {
    if (this.options.load === "languageOnly" || this.options.nonExplicitSupportedLngs) {
      code = this.getLanguagePartFromCode(code);
    }
    return !this.supportedLngs || !this.supportedLngs.length || this.supportedLngs.indexOf(code) > -1;
  }
  getBestMatchFromCodes(codes) {
    if (!codes) return null;
    let found;
    codes.forEach((code) => {
      if (found) return;
      const cleanedLng = this.formatLanguageCode(code);
      if (!this.options.supportedLngs || this.isSupportedCode(cleanedLng)) found = cleanedLng;
    });
    if (!found && this.options.supportedLngs) {
      codes.forEach((code) => {
        if (found) return;
        const lngOnly = this.getLanguagePartFromCode(code);
        if (this.isSupportedCode(lngOnly)) return found = lngOnly;
        found = this.options.supportedLngs.find((supportedLng) => {
          if (supportedLng === lngOnly) return supportedLng;
          if (supportedLng.indexOf("-") < 0 && lngOnly.indexOf("-") < 0) return;
          if (supportedLng.indexOf("-") > 0 && lngOnly.indexOf("-") < 0 && supportedLng.substring(0, supportedLng.indexOf("-")) === lngOnly) return supportedLng;
          if (supportedLng.indexOf(lngOnly) === 0 && lngOnly.length > 1) return supportedLng;
        });
      });
    }
    if (!found) found = this.getFallbackCodes(this.options.fallbackLng)[0];
    return found;
  }
  getFallbackCodes(fallbacks, code) {
    if (!fallbacks) return [];
    if (typeof fallbacks === "function") fallbacks = fallbacks(code);
    if (isString(fallbacks)) fallbacks = [fallbacks];
    if (Array.isArray(fallbacks)) return fallbacks;
    if (!code) return fallbacks.default || [];
    let found = fallbacks[code];
    if (!found) found = fallbacks[this.getScriptPartFromCode(code)];
    if (!found) found = fallbacks[this.formatLanguageCode(code)];
    if (!found) found = fallbacks[this.getLanguagePartFromCode(code)];
    if (!found) found = fallbacks.default;
    return found || [];
  }
  toResolveHierarchy(code, fallbackCode) {
    const fallbackCodes = this.getFallbackCodes(fallbackCode || this.options.fallbackLng || [], code);
    const codes = [];
    const addCode = (c2) => {
      if (!c2) return;
      if (this.isSupportedCode(c2)) {
        codes.push(c2);
      } else {
        this.logger.warn(`rejecting language code not found in supportedLngs: ${c2}`);
      }
    };
    if (isString(code) && (code.indexOf("-") > -1 || code.indexOf("_") > -1)) {
      if (this.options.load !== "languageOnly") addCode(this.formatLanguageCode(code));
      if (this.options.load !== "languageOnly" && this.options.load !== "currentOnly") addCode(this.getScriptPartFromCode(code));
      if (this.options.load !== "currentOnly") addCode(this.getLanguagePartFromCode(code));
    } else if (isString(code)) {
      addCode(this.formatLanguageCode(code));
    }
    fallbackCodes.forEach((fc) => {
      if (codes.indexOf(fc) < 0) addCode(this.formatLanguageCode(fc));
    });
    return codes;
  }
}
const suffixesOrder = {
  zero: 0,
  one: 1,
  two: 2,
  few: 3,
  many: 4,
  other: 5
};
const dummyRule = {
  select: (count) => count === 1 ? "one" : "other",
  resolvedOptions: () => ({
    pluralCategories: ["one", "other"]
  })
};
class PluralResolver {
  constructor(languageUtils) {
    let options = arguments.length > 1 && arguments[1] !== void 0 ? arguments[1] : {};
    this.languageUtils = languageUtils;
    this.options = options;
    this.logger = baseLogger.create("pluralResolver");
    this.pluralRulesCache = {};
  }
  addRule(lng, obj) {
    this.rules[lng] = obj;
  }
  clearCache() {
    this.pluralRulesCache = {};
  }
  getRule(code) {
    let options = arguments.length > 1 && arguments[1] !== void 0 ? arguments[1] : {};
    const cleanedCode = getCleanedCode(code === "dev" ? "en" : code);
    const type = options.ordinal ? "ordinal" : "cardinal";
    const cacheKey = JSON.stringify({
      cleanedCode,
      type
    });
    if (cacheKey in this.pluralRulesCache) {
      return this.pluralRulesCache[cacheKey];
    }
    let rule;
    try {
      rule = new Intl.PluralRules(cleanedCode, {
        type
      });
    } catch (err) {
      if (!Intl) {
        this.logger.error("No Intl support, please use an Intl polyfill!");
        return dummyRule;
      }
      if (!code.match(/-|_/)) return dummyRule;
      const lngPart = this.languageUtils.getLanguagePartFromCode(code);
      rule = this.getRule(lngPart, options);
    }
    this.pluralRulesCache[cacheKey] = rule;
    return rule;
  }
  needsPlural(code) {
    let options = arguments.length > 1 && arguments[1] !== void 0 ? arguments[1] : {};
    let rule = this.getRule(code, options);
    if (!rule) rule = this.getRule("dev", options);
    return (rule == null ? void 0 : rule.resolvedOptions().pluralCategories.length) > 1;
  }
  getPluralFormsOfKey(code, key) {
    let options = arguments.length > 2 && arguments[2] !== void 0 ? arguments[2] : {};
    return this.getSuffixes(code, options).map((suffix) => `${key}${suffix}`);
  }
  getSuffixes(code) {
    let options = arguments.length > 1 && arguments[1] !== void 0 ? arguments[1] : {};
    let rule = this.getRule(code, options);
    if (!rule) rule = this.getRule("dev", options);
    if (!rule) return [];
    return rule.resolvedOptions().pluralCategories.sort((pluralCategory1, pluralCategory2) => suffixesOrder[pluralCategory1] - suffixesOrder[pluralCategory2]).map((pluralCategory) => `${this.options.prepend}${options.ordinal ? `ordinal${this.options.prepend}` : ""}${pluralCategory}`);
  }
  getSuffix(code, count) {
    let options = arguments.length > 2 && arguments[2] !== void 0 ? arguments[2] : {};
    const rule = this.getRule(code, options);
    if (rule) {
      return `${this.options.prepend}${options.ordinal ? `ordinal${this.options.prepend}` : ""}${rule.select(count)}`;
    }
    this.logger.warn(`no plural rule found for: ${code}`);
    return this.getSuffix("dev", count, options);
  }
}
const deepFindWithDefaults = function(data, defaultData, key) {
  let keySeparator = arguments.length > 3 && arguments[3] !== void 0 ? arguments[3] : ".";
  let ignoreJSONStructure = arguments.length > 4 && arguments[4] !== void 0 ? arguments[4] : true;
  let path = getPathWithDefaults(data, defaultData, key);
  if (!path && ignoreJSONStructure && isString(key)) {
    path = deepFind(data, key, keySeparator);
    if (path === void 0) path = deepFind(defaultData, key, keySeparator);
  }
  return path;
};
const regexSafe = (val) => val.replace(/\$/g, "$$$$");
class Interpolator {
  constructor() {
    var _a2;
    let options = arguments.length > 0 && arguments[0] !== void 0 ? arguments[0] : {};
    this.logger = baseLogger.create("interpolator");
    this.options = options;
    this.format = ((_a2 = options == null ? void 0 : options.interpolation) == null ? void 0 : _a2.format) || ((value) => value);
    this.init(options);
  }
  init() {
    let options = arguments.length > 0 && arguments[0] !== void 0 ? arguments[0] : {};
    if (!options.interpolation) options.interpolation = {
      escapeValue: true
    };
    const {
      escape: escape$1,
      escapeValue,
      useRawValueToEscape,
      prefix,
      prefixEscaped,
      suffix,
      suffixEscaped,
      formatSeparator,
      unescapeSuffix,
      unescapePrefix,
      nestingPrefix,
      nestingPrefixEscaped,
      nestingSuffix,
      nestingSuffixEscaped,
      nestingOptionsSeparator,
      maxReplaces,
      alwaysFormat
    } = options.interpolation;
    this.escape = escape$1 !== void 0 ? escape$1 : escape;
    this.escapeValue = escapeValue !== void 0 ? escapeValue : true;
    this.useRawValueToEscape = useRawValueToEscape !== void 0 ? useRawValueToEscape : false;
    this.prefix = prefix ? regexEscape(prefix) : prefixEscaped || "{{";
    this.suffix = suffix ? regexEscape(suffix) : suffixEscaped || "}}";
    this.formatSeparator = formatSeparator || ",";
    this.unescapePrefix = unescapeSuffix ? "" : unescapePrefix || "-";
    this.unescapeSuffix = this.unescapePrefix ? "" : unescapeSuffix || "";
    this.nestingPrefix = nestingPrefix ? regexEscape(nestingPrefix) : nestingPrefixEscaped || regexEscape("$t(");
    this.nestingSuffix = nestingSuffix ? regexEscape(nestingSuffix) : nestingSuffixEscaped || regexEscape(")");
    this.nestingOptionsSeparator = nestingOptionsSeparator || ",";
    this.maxReplaces = maxReplaces || 1e3;
    this.alwaysFormat = alwaysFormat !== void 0 ? alwaysFormat : false;
    this.resetRegExp();
  }
  reset() {
    if (this.options) this.init(this.options);
  }
  resetRegExp() {
    const getOrResetRegExp = (existingRegExp, pattern) => {
      if ((existingRegExp == null ? void 0 : existingRegExp.source) === pattern) {
        existingRegExp.lastIndex = 0;
        return existingRegExp;
      }
      return new RegExp(pattern, "g");
    };
    this.regexp = getOrResetRegExp(this.regexp, `${this.prefix}(.+?)${this.suffix}`);
    this.regexpUnescape = getOrResetRegExp(this.regexpUnescape, `${this.prefix}${this.unescapePrefix}(.+?)${this.unescapeSuffix}${this.suffix}`);
    this.nestingRegexp = getOrResetRegExp(this.nestingRegexp, `${this.nestingPrefix}(.+?)${this.nestingSuffix}`);
  }
  interpolate(str, data, lng, options) {
    var _a2;
    let match;
    let value;
    let replaces;
    const defaultData = this.options && this.options.interpolation && this.options.interpolation.defaultVariables || {};
    const handleFormat = (key) => {
      if (key.indexOf(this.formatSeparator) < 0) {
        const path = deepFindWithDefaults(data, defaultData, key, this.options.keySeparator, this.options.ignoreJSONStructure);
        return this.alwaysFormat ? this.format(path, void 0, lng, {
          ...options,
          ...data,
          interpolationkey: key
        }) : path;
      }
      const p2 = key.split(this.formatSeparator);
      const k2 = p2.shift().trim();
      const f2 = p2.join(this.formatSeparator).trim();
      return this.format(deepFindWithDefaults(data, defaultData, k2, this.options.keySeparator, this.options.ignoreJSONStructure), f2, lng, {
        ...options,
        ...data,
        interpolationkey: k2
      });
    };
    this.resetRegExp();
    const missingInterpolationHandler = (options == null ? void 0 : options.missingInterpolationHandler) || this.options.missingInterpolationHandler;
    const skipOnVariables = ((_a2 = options == null ? void 0 : options.interpolation) == null ? void 0 : _a2.skipOnVariables) !== void 0 ? options.interpolation.skipOnVariables : this.options.interpolation.skipOnVariables;
    const todos = [{
      regex: this.regexpUnescape,
      safeValue: (val) => regexSafe(val)
    }, {
      regex: this.regexp,
      safeValue: (val) => this.escapeValue ? regexSafe(this.escape(val)) : regexSafe(val)
    }];
    todos.forEach((todo) => {
      replaces = 0;
      while (match = todo.regex.exec(str)) {
        const matchedVar = match[1].trim();
        value = handleFormat(matchedVar);
        if (value === void 0) {
          if (typeof missingInterpolationHandler === "function") {
            const temp = missingInterpolationHandler(str, match, options);
            value = isString(temp) ? temp : "";
          } else if (options && Object.prototype.hasOwnProperty.call(options, matchedVar)) {
            value = "";
          } else if (skipOnVariables) {
            value = match[0];
            continue;
          } else {
            this.logger.warn(`missed to pass in variable ${matchedVar} for interpolating ${str}`);
            value = "";
          }
        } else if (!isString(value) && !this.useRawValueToEscape) {
          value = makeString(value);
        }
        const safeValue = todo.safeValue(value);
        str = str.replace(match[0], safeValue);
        if (skipOnVariables) {
          todo.regex.lastIndex += value.length;
          todo.regex.lastIndex -= match[0].length;
        } else {
          todo.regex.lastIndex = 0;
        }
        replaces++;
        if (replaces >= this.maxReplaces) {
          break;
        }
      }
    });
    return str;
  }
  nest(str, fc) {
    let options = arguments.length > 2 && arguments[2] !== void 0 ? arguments[2] : {};
    let match;
    let value;
    let clonedOptions;
    const handleHasOptions = (key, inheritedOptions) => {
      const sep = this.nestingOptionsSeparator;
      if (key.indexOf(sep) < 0) return key;
      const c2 = key.split(new RegExp(`${sep}[ ]*{`));
      let optionsString = `{${c2[1]}`;
      key = c2[0];
      optionsString = this.interpolate(optionsString, clonedOptions);
      const matchedSingleQuotes = optionsString.match(/'/g);
      const matchedDoubleQuotes = optionsString.match(/"/g);
      if (((matchedSingleQuotes == null ? void 0 : matchedSingleQuotes.length) ?? 0) % 2 === 0 && !matchedDoubleQuotes || matchedDoubleQuotes.length % 2 !== 0) {
        optionsString = optionsString.replace(/'/g, '"');
      }
      try {
        clonedOptions = JSON.parse(optionsString);
        if (inheritedOptions) clonedOptions = {
          ...inheritedOptions,
          ...clonedOptions
        };
      } catch (e2) {
        this.logger.warn(`failed parsing options string in nesting for key ${key}`, e2);
        return `${key}${sep}${optionsString}`;
      }
      if (clonedOptions.defaultValue && clonedOptions.defaultValue.indexOf(this.prefix) > -1) delete clonedOptions.defaultValue;
      return key;
    };
    while (match = this.nestingRegexp.exec(str)) {
      let formatters = [];
      clonedOptions = {
        ...options
      };
      clonedOptions = clonedOptions.replace && !isString(clonedOptions.replace) ? clonedOptions.replace : clonedOptions;
      clonedOptions.applyPostProcessor = false;
      delete clonedOptions.defaultValue;
      let doReduce = false;
      if (match[0].indexOf(this.formatSeparator) !== -1 && !/{.*}/.test(match[1])) {
        const r2 = match[1].split(this.formatSeparator).map((elem) => elem.trim());
        match[1] = r2.shift();
        formatters = r2;
        doReduce = true;
      }
      value = fc(handleHasOptions.call(this, match[1].trim(), clonedOptions), clonedOptions);
      if (value && match[0] === str && !isString(value)) return value;
      if (!isString(value)) value = makeString(value);
      if (!value) {
        this.logger.warn(`missed to resolve ${match[1]} for nesting ${str}`);
        value = "";
      }
      if (doReduce) {
        value = formatters.reduce((v2, f2) => this.format(v2, f2, options.lng, {
          ...options,
          interpolationkey: match[1].trim()
        }), value.trim());
      }
      str = str.replace(match[0], value);
      this.regexp.lastIndex = 0;
    }
    return str;
  }
}
const parseFormatStr = (formatStr) => {
  let formatName = formatStr.toLowerCase().trim();
  const formatOptions = {};
  if (formatStr.indexOf("(") > -1) {
    const p2 = formatStr.split("(");
    formatName = p2[0].toLowerCase().trim();
    const optStr = p2[1].substring(0, p2[1].length - 1);
    if (formatName === "currency" && optStr.indexOf(":") < 0) {
      if (!formatOptions.currency) formatOptions.currency = optStr.trim();
    } else if (formatName === "relativetime" && optStr.indexOf(":") < 0) {
      if (!formatOptions.range) formatOptions.range = optStr.trim();
    } else {
      const opts = optStr.split(";");
      opts.forEach((opt) => {
        if (opt) {
          const [key, ...rest] = opt.split(":");
          const val = rest.join(":").trim().replace(/^'+|'+$/g, "");
          const trimmedKey = key.trim();
          if (!formatOptions[trimmedKey]) formatOptions[trimmedKey] = val;
          if (val === "false") formatOptions[trimmedKey] = false;
          if (val === "true") formatOptions[trimmedKey] = true;
          if (!isNaN(val)) formatOptions[trimmedKey] = parseInt(val, 10);
        }
      });
    }
  }
  return {
    formatName,
    formatOptions
  };
};
const createCachedFormatter = (fn) => {
  const cache = {};
  return (val, lng, options) => {
    let optForCache = options;
    if (options && options.interpolationkey && options.formatParams && options.formatParams[options.interpolationkey] && options[options.interpolationkey]) {
      optForCache = {
        ...optForCache,
        [options.interpolationkey]: void 0
      };
    }
    const key = lng + JSON.stringify(optForCache);
    let formatter = cache[key];
    if (!formatter) {
      formatter = fn(getCleanedCode(lng), options);
      cache[key] = formatter;
    }
    return formatter(val);
  };
};
class Formatter {
  constructor() {
    let options = arguments.length > 0 && arguments[0] !== void 0 ? arguments[0] : {};
    this.logger = baseLogger.create("formatter");
    this.options = options;
    this.formats = {
      number: createCachedFormatter((lng, opt) => {
        const formatter = new Intl.NumberFormat(lng, {
          ...opt
        });
        return (val) => formatter.format(val);
      }),
      currency: createCachedFormatter((lng, opt) => {
        const formatter = new Intl.NumberFormat(lng, {
          ...opt,
          style: "currency"
        });
        return (val) => formatter.format(val);
      }),
      datetime: createCachedFormatter((lng, opt) => {
        const formatter = new Intl.DateTimeFormat(lng, {
          ...opt
        });
        return (val) => formatter.format(val);
      }),
      relativetime: createCachedFormatter((lng, opt) => {
        const formatter = new Intl.RelativeTimeFormat(lng, {
          ...opt
        });
        return (val) => formatter.format(val, opt.range || "day");
      }),
      list: createCachedFormatter((lng, opt) => {
        const formatter = new Intl.ListFormat(lng, {
          ...opt
        });
        return (val) => formatter.format(val);
      })
    };
    this.init(options);
  }
  init(services) {
    let options = arguments.length > 1 && arguments[1] !== void 0 ? arguments[1] : {
      interpolation: {}
    };
    this.formatSeparator = options.interpolation.formatSeparator || ",";
  }
  add(name, fc) {
    this.formats[name.toLowerCase().trim()] = fc;
  }
  addCached(name, fc) {
    this.formats[name.toLowerCase().trim()] = createCachedFormatter(fc);
  }
  format(value, format, lng) {
    let options = arguments.length > 3 && arguments[3] !== void 0 ? arguments[3] : {};
    const formats = format.split(this.formatSeparator);
    if (formats.length > 1 && formats[0].indexOf("(") > 1 && formats[0].indexOf(")") < 0 && formats.find((f2) => f2.indexOf(")") > -1)) {
      const lastIndex = formats.findIndex((f2) => f2.indexOf(")") > -1);
      formats[0] = [formats[0], ...formats.splice(1, lastIndex)].join(this.formatSeparator);
    }
    const result = formats.reduce((mem, f2) => {
      var _a2;
      const {
        formatName,
        formatOptions
      } = parseFormatStr(f2);
      if (this.formats[formatName]) {
        let formatted = mem;
        try {
          const valOptions = ((_a2 = options == null ? void 0 : options.formatParams) == null ? void 0 : _a2[options.interpolationkey]) || {};
          const l2 = valOptions.locale || valOptions.lng || options.locale || options.lng || lng;
          formatted = this.formats[formatName](mem, l2, {
            ...formatOptions,
            ...options,
            ...valOptions
          });
        } catch (error) {
          this.logger.warn(error);
        }
        return formatted;
      } else {
        this.logger.warn(`there was no format function for ${formatName}`);
      }
      return mem;
    }, value);
    return result;
  }
}
const removePending = (q2, name) => {
  if (q2.pending[name] !== void 0) {
    delete q2.pending[name];
    q2.pendingCount--;
  }
};
class Connector extends EventEmitter {
  constructor(backend, store, services) {
    var _a2, _b;
    let options = arguments.length > 3 && arguments[3] !== void 0 ? arguments[3] : {};
    super();
    this.backend = backend;
    this.store = store;
    this.services = services;
    this.languageUtils = services.languageUtils;
    this.options = options;
    this.logger = baseLogger.create("backendConnector");
    this.waitingReads = [];
    this.maxParallelReads = options.maxParallelReads || 10;
    this.readingCalls = 0;
    this.maxRetries = options.maxRetries >= 0 ? options.maxRetries : 5;
    this.retryTimeout = options.retryTimeout >= 1 ? options.retryTimeout : 350;
    this.state = {};
    this.queue = [];
    (_b = (_a2 = this.backend) == null ? void 0 : _a2.init) == null ? void 0 : _b.call(_a2, services, options.backend, options);
  }
  queueLoad(languages, namespaces2, options, callback) {
    const toLoad = {};
    const pending2 = {};
    const toLoadLanguages = {};
    const toLoadNamespaces = {};
    languages.forEach((lng) => {
      let hasAllNamespaces = true;
      namespaces2.forEach((ns) => {
        const name = `${lng}|${ns}`;
        if (!options.reload && this.store.hasResourceBundle(lng, ns)) {
          this.state[name] = 2;
        } else if (this.state[name] < 0) ;
        else if (this.state[name] === 1) {
          if (pending2[name] === void 0) pending2[name] = true;
        } else {
          this.state[name] = 1;
          hasAllNamespaces = false;
          if (pending2[name] === void 0) pending2[name] = true;
          if (toLoad[name] === void 0) toLoad[name] = true;
          if (toLoadNamespaces[ns] === void 0) toLoadNamespaces[ns] = true;
        }
      });
      if (!hasAllNamespaces) toLoadLanguages[lng] = true;
    });
    if (Object.keys(toLoad).length || Object.keys(pending2).length) {
      this.queue.push({
        pending: pending2,
        pendingCount: Object.keys(pending2).length,
        loaded: {},
        errors: [],
        callback
      });
    }
    return {
      toLoad: Object.keys(toLoad),
      pending: Object.keys(pending2),
      toLoadLanguages: Object.keys(toLoadLanguages),
      toLoadNamespaces: Object.keys(toLoadNamespaces)
    };
  }
  loaded(name, err, data) {
    const s2 = name.split("|");
    const lng = s2[0];
    const ns = s2[1];
    if (err) this.emit("failedLoading", lng, ns, err);
    if (!err && data) {
      this.store.addResourceBundle(lng, ns, data, void 0, void 0, {
        skipCopy: true
      });
    }
    this.state[name] = err ? -1 : 2;
    if (err && data) this.state[name] = 0;
    const loaded = {};
    this.queue.forEach((q2) => {
      pushPath(q2.loaded, [lng], ns);
      removePending(q2, name);
      if (err) q2.errors.push(err);
      if (q2.pendingCount === 0 && !q2.done) {
        Object.keys(q2.loaded).forEach((l2) => {
          if (!loaded[l2]) loaded[l2] = {};
          const loadedKeys = q2.loaded[l2];
          if (loadedKeys.length) {
            loadedKeys.forEach((n2) => {
              if (loaded[l2][n2] === void 0) loaded[l2][n2] = true;
            });
          }
        });
        q2.done = true;
        if (q2.errors.length) {
          q2.callback(q2.errors);
        } else {
          q2.callback();
        }
      }
    });
    this.emit("loaded", loaded);
    this.queue = this.queue.filter((q2) => !q2.done);
  }
  read(lng, ns, fcName) {
    let tried = arguments.length > 3 && arguments[3] !== void 0 ? arguments[3] : 0;
    let wait = arguments.length > 4 && arguments[4] !== void 0 ? arguments[4] : this.retryTimeout;
    let callback = arguments.length > 5 ? arguments[5] : void 0;
    if (!lng.length) return callback(null, {});
    if (this.readingCalls >= this.maxParallelReads) {
      this.waitingReads.push({
        lng,
        ns,
        fcName,
        tried,
        wait,
        callback
      });
      return;
    }
    this.readingCalls++;
    const resolver = (err, data) => {
      this.readingCalls--;
      if (this.waitingReads.length > 0) {
        const next = this.waitingReads.shift();
        this.read(next.lng, next.ns, next.fcName, next.tried, next.wait, next.callback);
      }
      if (err && data && tried < this.maxRetries) {
        setTimeout(() => {
          this.read.call(this, lng, ns, fcName, tried + 1, wait * 2, callback);
        }, wait);
        return;
      }
      callback(err, data);
    };
    const fc = this.backend[fcName].bind(this.backend);
    if (fc.length === 2) {
      try {
        const r2 = fc(lng, ns);
        if (r2 && typeof r2.then === "function") {
          r2.then((data) => resolver(null, data)).catch(resolver);
        } else {
          resolver(null, r2);
        }
      } catch (err) {
        resolver(err);
      }
      return;
    }
    return fc(lng, ns, resolver);
  }
  prepareLoading(languages, namespaces2) {
    let options = arguments.length > 2 && arguments[2] !== void 0 ? arguments[2] : {};
    let callback = arguments.length > 3 ? arguments[3] : void 0;
    if (!this.backend) {
      this.logger.warn("No backend was added via i18next.use. Will not load resources.");
      return callback && callback();
    }
    if (isString(languages)) languages = this.languageUtils.toResolveHierarchy(languages);
    if (isString(namespaces2)) namespaces2 = [namespaces2];
    const toLoad = this.queueLoad(languages, namespaces2, options, callback);
    if (!toLoad.toLoad.length) {
      if (!toLoad.pending.length) callback();
      return null;
    }
    toLoad.toLoad.forEach((name) => {
      this.loadOne(name);
    });
  }
  load(languages, namespaces2, callback) {
    this.prepareLoading(languages, namespaces2, {}, callback);
  }
  reload(languages, namespaces2, callback) {
    this.prepareLoading(languages, namespaces2, {
      reload: true
    }, callback);
  }
  loadOne(name) {
    let prefix = arguments.length > 1 && arguments[1] !== void 0 ? arguments[1] : "";
    const s2 = name.split("|");
    const lng = s2[0];
    const ns = s2[1];
    this.read(lng, ns, "read", void 0, void 0, (err, data) => {
      if (err) this.logger.warn(`${prefix}loading namespace ${ns} for language ${lng} failed`, err);
      if (!err && data) this.logger.log(`${prefix}loaded namespace ${ns} for language ${lng}`, data);
      this.loaded(name, err, data);
    });
  }
  saveMissing(languages, namespace, key, fallbackValue, isUpdate) {
    var _a2, _b, _c, _d, _e2;
    let options = arguments.length > 5 && arguments[5] !== void 0 ? arguments[5] : {};
    let clb = arguments.length > 6 && arguments[6] !== void 0 ? arguments[6] : () => {
    };
    if (((_b = (_a2 = this.services) == null ? void 0 : _a2.utils) == null ? void 0 : _b.hasLoadedNamespace) && !((_d = (_c = this.services) == null ? void 0 : _c.utils) == null ? void 0 : _d.hasLoadedNamespace(namespace))) {
      this.logger.warn(`did not save key "${key}" as the namespace "${namespace}" was not yet loaded`, "This means something IS WRONG in your setup. You access the t function before i18next.init / i18next.loadNamespace / i18next.changeLanguage was done. Wait for the callback or Promise to resolve before accessing it!!!");
      return;
    }
    if (key === void 0 || key === null || key === "") return;
    if ((_e2 = this.backend) == null ? void 0 : _e2.create) {
      const opts = {
        ...options,
        isUpdate
      };
      const fc = this.backend.create.bind(this.backend);
      if (fc.length < 6) {
        try {
          let r2;
          if (fc.length === 5) {
            r2 = fc(languages, namespace, key, fallbackValue, opts);
          } else {
            r2 = fc(languages, namespace, key, fallbackValue);
          }
          if (r2 && typeof r2.then === "function") {
            r2.then((data) => clb(null, data)).catch(clb);
          } else {
            clb(null, r2);
          }
        } catch (err) {
          clb(err);
        }
      } else {
        fc(languages, namespace, key, fallbackValue, clb, opts);
      }
    }
    if (!languages || !languages[0]) return;
    this.store.addResource(languages[0], namespace, key, fallbackValue);
  }
}
const get = () => ({
  debug: false,
  initAsync: true,
  ns: ["translation"],
  defaultNS: ["translation"],
  fallbackLng: ["dev"],
  fallbackNS: false,
  supportedLngs: false,
  nonExplicitSupportedLngs: false,
  load: "all",
  preload: false,
  simplifyPluralSuffix: true,
  keySeparator: ".",
  nsSeparator: ":",
  pluralSeparator: "_",
  contextSeparator: "_",
  partialBundledLanguages: false,
  saveMissing: false,
  updateMissing: false,
  saveMissingTo: "fallback",
  saveMissingPlurals: true,
  missingKeyHandler: false,
  missingInterpolationHandler: false,
  postProcess: false,
  postProcessPassResolved: false,
  returnNull: false,
  returnEmptyString: true,
  returnObjects: false,
  joinArrays: false,
  returnedObjectHandler: false,
  parseMissingKeyHandler: false,
  appendNamespaceToMissingKey: false,
  appendNamespaceToCIMode: false,
  overloadTranslationOptionHandler: (args) => {
    let ret = {};
    if (typeof args[1] === "object") ret = args[1];
    if (isString(args[1])) ret.defaultValue = args[1];
    if (isString(args[2])) ret.tDescription = args[2];
    if (typeof args[2] === "object" || typeof args[3] === "object") {
      const options = args[3] || args[2];
      Object.keys(options).forEach((key) => {
        ret[key] = options[key];
      });
    }
    return ret;
  },
  interpolation: {
    escapeValue: true,
    format: (value) => value,
    prefix: "{{",
    suffix: "}}",
    formatSeparator: ",",
    unescapePrefix: "-",
    nestingPrefix: "$t(",
    nestingSuffix: ")",
    nestingOptionsSeparator: ",",
    maxReplaces: 1e3,
    skipOnVariables: true
  }
});
const transformOptions = (options) => {
  var _a2, _b;
  if (isString(options.ns)) options.ns = [options.ns];
  if (isString(options.fallbackLng)) options.fallbackLng = [options.fallbackLng];
  if (isString(options.fallbackNS)) options.fallbackNS = [options.fallbackNS];
  if (((_b = (_a2 = options.supportedLngs) == null ? void 0 : _a2.indexOf) == null ? void 0 : _b.call(_a2, "cimode")) < 0) {
    options.supportedLngs = options.supportedLngs.concat(["cimode"]);
  }
  if (typeof options.initImmediate === "boolean") options.initAsync = options.initImmediate;
  return options;
};
const noop = () => {
};
const bindMemberFunctions = (inst) => {
  const mems = Object.getOwnPropertyNames(Object.getPrototypeOf(inst));
  mems.forEach((mem) => {
    if (typeof inst[mem] === "function") {
      inst[mem] = inst[mem].bind(inst);
    }
  });
};
class I18n extends EventEmitter {
  constructor() {
    let options = arguments.length > 0 && arguments[0] !== void 0 ? arguments[0] : {};
    let callback = arguments.length > 1 ? arguments[1] : void 0;
    super();
    this.options = transformOptions(options);
    this.services = {};
    this.logger = baseLogger;
    this.modules = {
      external: []
    };
    bindMemberFunctions(this);
    if (callback && !this.isInitialized && !options.isClone) {
      if (!this.options.initAsync) {
        this.init(options, callback);
        return this;
      }
      setTimeout(() => {
        this.init(options, callback);
      }, 0);
    }
  }
  init() {
    var _this = this;
    let options = arguments.length > 0 && arguments[0] !== void 0 ? arguments[0] : {};
    let callback = arguments.length > 1 ? arguments[1] : void 0;
    this.isInitializing = true;
    if (typeof options === "function") {
      callback = options;
      options = {};
    }
    if (options.defaultNS == null && options.ns) {
      if (isString(options.ns)) {
        options.defaultNS = options.ns;
      } else if (options.ns.indexOf("translation") < 0) {
        options.defaultNS = options.ns[0];
      }
    }
    const defOpts = get();
    this.options = {
      ...defOpts,
      ...this.options,
      ...transformOptions(options)
    };
    this.options.interpolation = {
      ...defOpts.interpolation,
      ...this.options.interpolation
    };
    if (options.keySeparator !== void 0) {
      this.options.userDefinedKeySeparator = options.keySeparator;
    }
    if (options.nsSeparator !== void 0) {
      this.options.userDefinedNsSeparator = options.nsSeparator;
    }
    const createClassOnDemand = (ClassOrObject) => {
      if (!ClassOrObject) return null;
      if (typeof ClassOrObject === "function") return new ClassOrObject();
      return ClassOrObject;
    };
    if (!this.options.isClone) {
      if (this.modules.logger) {
        baseLogger.init(createClassOnDemand(this.modules.logger), this.options);
      } else {
        baseLogger.init(null, this.options);
      }
      let formatter;
      if (this.modules.formatter) {
        formatter = this.modules.formatter;
      } else {
        formatter = Formatter;
      }
      const lu = new LanguageUtil(this.options);
      this.store = new ResourceStore(this.options.resources, this.options);
      const s2 = this.services;
      s2.logger = baseLogger;
      s2.resourceStore = this.store;
      s2.languageUtils = lu;
      s2.pluralResolver = new PluralResolver(lu, {
        prepend: this.options.pluralSeparator,
        simplifyPluralSuffix: this.options.simplifyPluralSuffix
      });
      if (formatter && (!this.options.interpolation.format || this.options.interpolation.format === defOpts.interpolation.format)) {
        s2.formatter = createClassOnDemand(formatter);
        s2.formatter.init(s2, this.options);
        this.options.interpolation.format = s2.formatter.format.bind(s2.formatter);
      }
      s2.interpolator = new Interpolator(this.options);
      s2.utils = {
        hasLoadedNamespace: this.hasLoadedNamespace.bind(this)
      };
      s2.backendConnector = new Connector(createClassOnDemand(this.modules.backend), s2.resourceStore, s2, this.options);
      s2.backendConnector.on("*", function(event) {
        for (var _len = arguments.length, args = new Array(_len > 1 ? _len - 1 : 0), _key = 1; _key < _len; _key++) {
          args[_key - 1] = arguments[_key];
        }
        _this.emit(event, ...args);
      });
      if (this.modules.languageDetector) {
        s2.languageDetector = createClassOnDemand(this.modules.languageDetector);
        if (s2.languageDetector.init) s2.languageDetector.init(s2, this.options.detection, this.options);
      }
      if (this.modules.i18nFormat) {
        s2.i18nFormat = createClassOnDemand(this.modules.i18nFormat);
        if (s2.i18nFormat.init) s2.i18nFormat.init(this);
      }
      this.translator = new Translator(this.services, this.options);
      this.translator.on("*", function(event) {
        for (var _len2 = arguments.length, args = new Array(_len2 > 1 ? _len2 - 1 : 0), _key2 = 1; _key2 < _len2; _key2++) {
          args[_key2 - 1] = arguments[_key2];
        }
        _this.emit(event, ...args);
      });
      this.modules.external.forEach((m2) => {
        if (m2.init) m2.init(this);
      });
    }
    this.format = this.options.interpolation.format;
    if (!callback) callback = noop;
    if (this.options.fallbackLng && !this.services.languageDetector && !this.options.lng) {
      const codes = this.services.languageUtils.getFallbackCodes(this.options.fallbackLng);
      if (codes.length > 0 && codes[0] !== "dev") this.options.lng = codes[0];
    }
    if (!this.services.languageDetector && !this.options.lng) {
      this.logger.warn("init: no languageDetector is used and no lng is defined");
    }
    const storeApi = ["getResource", "hasResourceBundle", "getResourceBundle", "getDataByLanguage"];
    storeApi.forEach((fcName) => {
      this[fcName] = function() {
        return _this.store[fcName](...arguments);
      };
    });
    const storeApiChained = ["addResource", "addResources", "addResourceBundle", "removeResourceBundle"];
    storeApiChained.forEach((fcName) => {
      this[fcName] = function() {
        _this.store[fcName](...arguments);
        return _this;
      };
    });
    const deferred = defer();
    const load = () => {
      const finish = (err, t2) => {
        this.isInitializing = false;
        if (this.isInitialized && !this.initializedStoreOnce) this.logger.warn("init: i18next is already initialized. You should call init just once!");
        this.isInitialized = true;
        if (!this.options.isClone) this.logger.log("initialized", this.options);
        this.emit("initialized", this.options);
        deferred.resolve(t2);
        callback(err, t2);
      };
      if (this.languages && !this.isInitialized) return finish(null, this.t.bind(this));
      this.changeLanguage(this.options.lng, finish);
    };
    if (this.options.resources || !this.options.initAsync) {
      load();
    } else {
      setTimeout(load, 0);
    }
    return deferred;
  }
  loadResources(language) {
    var _a2, _b;
    let callback = arguments.length > 1 && arguments[1] !== void 0 ? arguments[1] : noop;
    let usedCallback = callback;
    const usedLng = isString(language) ? language : this.language;
    if (typeof language === "function") usedCallback = language;
    if (!this.options.resources || this.options.partialBundledLanguages) {
      if ((usedLng == null ? void 0 : usedLng.toLowerCase()) === "cimode" && (!this.options.preload || this.options.preload.length === 0)) return usedCallback();
      const toLoad = [];
      const append = (lng) => {
        if (!lng) return;
        if (lng === "cimode") return;
        const lngs = this.services.languageUtils.toResolveHierarchy(lng);
        lngs.forEach((l2) => {
          if (l2 === "cimode") return;
          if (toLoad.indexOf(l2) < 0) toLoad.push(l2);
        });
      };
      if (!usedLng) {
        const fallbacks = this.services.languageUtils.getFallbackCodes(this.options.fallbackLng);
        fallbacks.forEach((l2) => append(l2));
      } else {
        append(usedLng);
      }
      (_b = (_a2 = this.options.preload) == null ? void 0 : _a2.forEach) == null ? void 0 : _b.call(_a2, (l2) => append(l2));
      this.services.backendConnector.load(toLoad, this.options.ns, (e2) => {
        if (!e2 && !this.resolvedLanguage && this.language) this.setResolvedLanguage(this.language);
        usedCallback(e2);
      });
    } else {
      usedCallback(null);
    }
  }
  reloadResources(lngs, ns, callback) {
    const deferred = defer();
    if (typeof lngs === "function") {
      callback = lngs;
      lngs = void 0;
    }
    if (typeof ns === "function") {
      callback = ns;
      ns = void 0;
    }
    if (!lngs) lngs = this.languages;
    if (!ns) ns = this.options.ns;
    if (!callback) callback = noop;
    this.services.backendConnector.reload(lngs, ns, (err) => {
      deferred.resolve();
      callback(err);
    });
    return deferred;
  }
  use(module) {
    if (!module) throw new Error("You are passing an undefined module! Please check the object you are passing to i18next.use()");
    if (!module.type) throw new Error("You are passing a wrong module! Please check the object you are passing to i18next.use()");
    if (module.type === "backend") {
      this.modules.backend = module;
    }
    if (module.type === "logger" || module.log && module.warn && module.error) {
      this.modules.logger = module;
    }
    if (module.type === "languageDetector") {
      this.modules.languageDetector = module;
    }
    if (module.type === "i18nFormat") {
      this.modules.i18nFormat = module;
    }
    if (module.type === "postProcessor") {
      postProcessor.addPostProcessor(module);
    }
    if (module.type === "formatter") {
      this.modules.formatter = module;
    }
    if (module.type === "3rdParty") {
      this.modules.external.push(module);
    }
    return this;
  }
  setResolvedLanguage(l2) {
    if (!l2 || !this.languages) return;
    if (["cimode", "dev"].indexOf(l2) > -1) return;
    for (let li = 0; li < this.languages.length; li++) {
      const lngInLngs = this.languages[li];
      if (["cimode", "dev"].indexOf(lngInLngs) > -1) continue;
      if (this.store.hasLanguageSomeTranslations(lngInLngs)) {
        this.resolvedLanguage = lngInLngs;
        break;
      }
    }
  }
  changeLanguage(lng, callback) {
    var _this2 = this;
    this.isLanguageChangingTo = lng;
    const deferred = defer();
    this.emit("languageChanging", lng);
    const setLngProps = (l2) => {
      this.language = l2;
      this.languages = this.services.languageUtils.toResolveHierarchy(l2);
      this.resolvedLanguage = void 0;
      this.setResolvedLanguage(l2);
    };
    const done = (err, l2) => {
      if (l2) {
        setLngProps(l2);
        this.translator.changeLanguage(l2);
        this.isLanguageChangingTo = void 0;
        this.emit("languageChanged", l2);
        this.logger.log("languageChanged", l2);
      } else {
        this.isLanguageChangingTo = void 0;
      }
      deferred.resolve(function() {
        return _this2.t(...arguments);
      });
      if (callback) callback(err, function() {
        return _this2.t(...arguments);
      });
    };
    const setLng = (lngs) => {
      var _a2, _b;
      if (!lng && !lngs && this.services.languageDetector) lngs = [];
      const l2 = isString(lngs) ? lngs : this.services.languageUtils.getBestMatchFromCodes(lngs);
      if (l2) {
        if (!this.language) {
          setLngProps(l2);
        }
        if (!this.translator.language) this.translator.changeLanguage(l2);
        (_b = (_a2 = this.services.languageDetector) == null ? void 0 : _a2.cacheUserLanguage) == null ? void 0 : _b.call(_a2, l2);
      }
      this.loadResources(l2, (err) => {
        done(err, l2);
      });
    };
    if (!lng && this.services.languageDetector && !this.services.languageDetector.async) {
      setLng(this.services.languageDetector.detect());
    } else if (!lng && this.services.languageDetector && this.services.languageDetector.async) {
      if (this.services.languageDetector.detect.length === 0) {
        this.services.languageDetector.detect().then(setLng);
      } else {
        this.services.languageDetector.detect(setLng);
      }
    } else {
      setLng(lng);
    }
    return deferred;
  }
  getFixedT(lng, ns, keyPrefix) {
    var _this3 = this;
    const fixedT = function(key, opts) {
      let options;
      if (typeof opts !== "object") {
        for (var _len3 = arguments.length, rest = new Array(_len3 > 2 ? _len3 - 2 : 0), _key3 = 2; _key3 < _len3; _key3++) {
          rest[_key3 - 2] = arguments[_key3];
        }
        options = _this3.options.overloadTranslationOptionHandler([key, opts].concat(rest));
      } else {
        options = {
          ...opts
        };
      }
      options.lng = options.lng || fixedT.lng;
      options.lngs = options.lngs || fixedT.lngs;
      options.ns = options.ns || fixedT.ns;
      if (options.keyPrefix !== "") options.keyPrefix = options.keyPrefix || keyPrefix || fixedT.keyPrefix;
      const keySeparator = _this3.options.keySeparator || ".";
      let resultKey;
      if (options.keyPrefix && Array.isArray(key)) {
        resultKey = key.map((k2) => `${options.keyPrefix}${keySeparator}${k2}`);
      } else {
        resultKey = options.keyPrefix ? `${options.keyPrefix}${keySeparator}${key}` : key;
      }
      return _this3.t(resultKey, options);
    };
    if (isString(lng)) {
      fixedT.lng = lng;
    } else {
      fixedT.lngs = lng;
    }
    fixedT.ns = ns;
    fixedT.keyPrefix = keyPrefix;
    return fixedT;
  }
  t() {
    var _a2;
    for (var _len4 = arguments.length, args = new Array(_len4), _key4 = 0; _key4 < _len4; _key4++) {
      args[_key4] = arguments[_key4];
    }
    return (_a2 = this.translator) == null ? void 0 : _a2.translate(...args);
  }
  exists() {
    var _a2;
    for (var _len5 = arguments.length, args = new Array(_len5), _key5 = 0; _key5 < _len5; _key5++) {
      args[_key5] = arguments[_key5];
    }
    return (_a2 = this.translator) == null ? void 0 : _a2.exists(...args);
  }
  setDefaultNamespace(ns) {
    this.options.defaultNS = ns;
  }
  hasLoadedNamespace(ns) {
    let options = arguments.length > 1 && arguments[1] !== void 0 ? arguments[1] : {};
    if (!this.isInitialized) {
      this.logger.warn("hasLoadedNamespace: i18next was not initialized", this.languages);
      return false;
    }
    if (!this.languages || !this.languages.length) {
      this.logger.warn("hasLoadedNamespace: i18n.languages were undefined or empty", this.languages);
      return false;
    }
    const lng = options.lng || this.resolvedLanguage || this.languages[0];
    const fallbackLng = this.options ? this.options.fallbackLng : false;
    const lastLng = this.languages[this.languages.length - 1];
    if (lng.toLowerCase() === "cimode") return true;
    const loadNotPending = (l2, n2) => {
      const loadState = this.services.backendConnector.state[`${l2}|${n2}`];
      return loadState === -1 || loadState === 0 || loadState === 2;
    };
    if (options.precheck) {
      const preResult = options.precheck(this, loadNotPending);
      if (preResult !== void 0) return preResult;
    }
    if (this.hasResourceBundle(lng, ns)) return true;
    if (!this.services.backendConnector.backend || this.options.resources && !this.options.partialBundledLanguages) return true;
    if (loadNotPending(lng, ns) && (!fallbackLng || loadNotPending(lastLng, ns))) return true;
    return false;
  }
  loadNamespaces(ns, callback) {
    const deferred = defer();
    if (!this.options.ns) {
      if (callback) callback();
      return Promise.resolve();
    }
    if (isString(ns)) ns = [ns];
    ns.forEach((n2) => {
      if (this.options.ns.indexOf(n2) < 0) this.options.ns.push(n2);
    });
    this.loadResources((err) => {
      deferred.resolve();
      if (callback) callback(err);
    });
    return deferred;
  }
  loadLanguages(lngs, callback) {
    const deferred = defer();
    if (isString(lngs)) lngs = [lngs];
    const preloaded = this.options.preload || [];
    const newLngs = lngs.filter((lng) => preloaded.indexOf(lng) < 0 && this.services.languageUtils.isSupportedCode(lng));
    if (!newLngs.length) {
      if (callback) callback();
      return Promise.resolve();
    }
    this.options.preload = preloaded.concat(newLngs);
    this.loadResources((err) => {
      deferred.resolve();
      if (callback) callback(err);
    });
    return deferred;
  }
  dir(lng) {
    var _a2, _b;
    if (!lng) lng = this.resolvedLanguage || (((_a2 = this.languages) == null ? void 0 : _a2.length) > 0 ? this.languages[0] : this.language);
    if (!lng) return "rtl";
    const rtlLngs = ["ar", "shu", "sqr", "ssh", "xaa", "yhd", "yud", "aao", "abh", "abv", "acm", "acq", "acw", "acx", "acy", "adf", "ads", "aeb", "aec", "afb", "ajp", "apc", "apd", "arb", "arq", "ars", "ary", "arz", "auz", "avl", "ayh", "ayl", "ayn", "ayp", "bbz", "pga", "he", "iw", "ps", "pbt", "pbu", "pst", "prp", "prd", "ug", "ur", "ydd", "yds", "yih", "ji", "yi", "hbo", "men", "xmn", "fa", "jpr", "peo", "pes", "prs", "dv", "sam", "ckb"];
    const languageUtils = ((_b = this.services) == null ? void 0 : _b.languageUtils) || new LanguageUtil(get());
    return rtlLngs.indexOf(languageUtils.getLanguagePartFromCode(lng)) > -1 || lng.toLowerCase().indexOf("-arab") > 1 ? "rtl" : "ltr";
  }
  static createInstance() {
    let options = arguments.length > 0 && arguments[0] !== void 0 ? arguments[0] : {};
    let callback = arguments.length > 1 ? arguments[1] : void 0;
    return new I18n(options, callback);
  }
  cloneInstance() {
    let options = arguments.length > 0 && arguments[0] !== void 0 ? arguments[0] : {};
    let callback = arguments.length > 1 && arguments[1] !== void 0 ? arguments[1] : noop;
    const forkResourceStore = options.forkResourceStore;
    if (forkResourceStore) delete options.forkResourceStore;
    const mergedOptions = {
      ...this.options,
      ...options,
      ...{
        isClone: true
      }
    };
    const clone = new I18n(mergedOptions);
    if (options.debug !== void 0 || options.prefix !== void 0) {
      clone.logger = clone.logger.clone(options);
    }
    const membersToCopy = ["store", "services", "language"];
    membersToCopy.forEach((m2) => {
      clone[m2] = this[m2];
    });
    clone.services = {
      ...this.services
    };
    clone.services.utils = {
      hasLoadedNamespace: clone.hasLoadedNamespace.bind(clone)
    };
    if (forkResourceStore) {
      const clonedData = Object.keys(this.store.data).reduce((prev, l2) => {
        prev[l2] = {
          ...this.store.data[l2]
        };
        return Object.keys(prev[l2]).reduce((acc, n2) => {
          acc[n2] = {
            ...prev[l2][n2]
          };
          return acc;
        }, {});
      }, {});
      clone.store = new ResourceStore(clonedData, mergedOptions);
      clone.services.resourceStore = clone.store;
    }
    clone.translator = new Translator(clone.services, mergedOptions);
    clone.translator.on("*", function(event) {
      for (var _len6 = arguments.length, args = new Array(_len6 > 1 ? _len6 - 1 : 0), _key6 = 1; _key6 < _len6; _key6++) {
        args[_key6 - 1] = arguments[_key6];
      }
      clone.emit(event, ...args);
    });
    clone.init(mergedOptions, callback);
    clone.translator.options = mergedOptions;
    clone.translator.backendConnector.services.utils = {
      hasLoadedNamespace: clone.hasLoadedNamespace.bind(clone)
    };
    return clone;
  }
  toJSON() {
    return {
      options: this.options,
      store: this.store,
      language: this.language,
      languages: this.languages,
      resolvedLanguage: this.resolvedLanguage
    };
  }
}
const instance = I18n.createInstance();
instance.createInstance = I18n.createInstance;
instance.createInstance;
instance.dir;
instance.init;
instance.loadResources;
instance.reloadResources;
instance.use;
instance.changeLanguage;
instance.getFixedT;
instance.t;
instance.exists;
instance.setDefaultNamespace;
instance.hasLoadedNamespace;
instance.loadNamespaces;
instance.loadLanguages;
const STORAGE_KEY = "moltis-locale";
let initPromise = null;
const SUPPORTED_LOCALES = /* @__PURE__ */ new Set(["en", "fr", "zh"]);
const supportedLocales = Object.freeze(["en", "fr", "zh"]);
function normalizeLocaleTag(value) {
  if (!value) return "en";
  let tag = String(value).trim().replace("_", "-");
  if (!tag) return "en";
  const idx = tag.indexOf("-");
  if (idx !== -1) {
    tag = tag.slice(0, idx);
  }
  return tag.toLowerCase();
}
function resolveSupportedLocale(value) {
  const normalized = normalizeLocaleTag(value);
  if (SUPPORTED_LOCALES.has(normalized)) return normalized;
  return "en";
}
function getPreferredLocale() {
  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored) {
    return resolveSupportedLocale(stored);
  }
  return resolveSupportedLocale(navigator.language || "en");
}
const locale = y$1(getPreferredLocale());
const namespaces = {
  common: (lng) => __variableDynamicImportRuntimeHelper(/* @__PURE__ */ Object.assign({ "./locales/en/common.ts": () => __vitePreload(() => import("./common.js"), true ? [] : void 0), "./locales/fr/common.ts": () => __vitePreload(() => import("./common2.js"), true ? [] : void 0), "./locales/zh/common.ts": () => __vitePreload(() => import("./common3.js"), true ? [] : void 0) }), `./locales/${lng}/common.ts`, 4),
  errors: (lng) => __variableDynamicImportRuntimeHelper(/* @__PURE__ */ Object.assign({ "./locales/en/errors.ts": () => __vitePreload(() => import("./errors.js"), true ? [] : void 0), "./locales/fr/errors.ts": () => __vitePreload(() => import("./errors2.js"), true ? [] : void 0), "./locales/zh/errors.ts": () => __vitePreload(() => import("./errors3.js"), true ? [] : void 0) }), `./locales/${lng}/errors.ts`, 4),
  settings: (lng) => __variableDynamicImportRuntimeHelper(/* @__PURE__ */ Object.assign({ "./locales/en/settings.ts": () => __vitePreload(() => import("./settings.js"), true ? [] : void 0), "./locales/fr/settings.ts": () => __vitePreload(() => import("./settings2.js"), true ? [] : void 0), "./locales/zh/settings.ts": () => __vitePreload(() => import("./settings3.js"), true ? [] : void 0) }), `./locales/${lng}/settings.ts`, 4),
  providers: (lng) => __variableDynamicImportRuntimeHelper(/* @__PURE__ */ Object.assign({ "./locales/en/providers.ts": () => __vitePreload(() => import("./providers.js"), true ? [] : void 0), "./locales/fr/providers.ts": () => __vitePreload(() => import("./providers2.js"), true ? [] : void 0), "./locales/zh/providers.ts": () => __vitePreload(() => import("./providers3.js"), true ? [] : void 0) }), `./locales/${lng}/providers.ts`, 4),
  chat: (lng) => __variableDynamicImportRuntimeHelper(/* @__PURE__ */ Object.assign({ "./locales/en/chat.ts": () => __vitePreload(() => import("./chat.js"), true ? [] : void 0), "./locales/fr/chat.ts": () => __vitePreload(() => import("./chat2.js"), true ? [] : void 0), "./locales/zh/chat.ts": () => __vitePreload(() => import("./chat3.js"), true ? [] : void 0) }), `./locales/${lng}/chat.ts`, 4),
  onboarding: (lng) => __variableDynamicImportRuntimeHelper(/* @__PURE__ */ Object.assign({ "./locales/en/onboarding.ts": () => __vitePreload(() => import("./onboarding.js"), true ? [] : void 0), "./locales/fr/onboarding.ts": () => __vitePreload(() => import("./onboarding2.js"), true ? [] : void 0), "./locales/zh/onboarding.ts": () => __vitePreload(() => import("./onboarding3.js"), true ? [] : void 0) }), `./locales/${lng}/onboarding.ts`, 4),
  login: (lng) => __variableDynamicImportRuntimeHelper(/* @__PURE__ */ Object.assign({ "./locales/en/login.ts": () => __vitePreload(() => import("./login.js"), true ? [] : void 0), "./locales/fr/login.ts": () => __vitePreload(() => import("./login2.js"), true ? [] : void 0), "./locales/zh/login.ts": () => __vitePreload(() => import("./login3.js"), true ? [] : void 0) }), `./locales/${lng}/login.ts`, 4),
  crons: (lng) => __variableDynamicImportRuntimeHelper(/* @__PURE__ */ Object.assign({ "./locales/en/crons.ts": () => __vitePreload(() => import("./crons.js"), true ? [] : void 0), "./locales/fr/crons.ts": () => __vitePreload(() => import("./crons2.js"), true ? [] : void 0), "./locales/zh/crons.ts": () => __vitePreload(() => import("./crons3.js"), true ? [] : void 0) }), `./locales/${lng}/crons.ts`, 4),
  mcp: (lng) => __variableDynamicImportRuntimeHelper(/* @__PURE__ */ Object.assign({ "./locales/en/mcp.ts": () => __vitePreload(() => import("./mcp.js"), true ? [] : void 0), "./locales/fr/mcp.ts": () => __vitePreload(() => import("./mcp2.js"), true ? [] : void 0), "./locales/zh/mcp.ts": () => __vitePreload(() => import("./mcp3.js"), true ? [] : void 0) }), `./locales/${lng}/mcp.ts`, 4),
  skills: (lng) => __variableDynamicImportRuntimeHelper(/* @__PURE__ */ Object.assign({ "./locales/en/skills.ts": () => __vitePreload(() => import("./skills.js"), true ? [] : void 0), "./locales/fr/skills.ts": () => __vitePreload(() => import("./skills2.js"), true ? [] : void 0), "./locales/zh/skills.ts": () => __vitePreload(() => import("./skills3.js"), true ? [] : void 0) }), `./locales/${lng}/skills.ts`, 4),
  channels: (lng) => __variableDynamicImportRuntimeHelper(/* @__PURE__ */ Object.assign({ "./locales/en/channels.ts": () => __vitePreload(() => import("./channels.js"), true ? [] : void 0), "./locales/fr/channels.ts": () => __vitePreload(() => import("./channels2.js"), true ? [] : void 0), "./locales/zh/channels.ts": () => __vitePreload(() => import("./channels3.js"), true ? [] : void 0) }), `./locales/${lng}/channels.ts`, 4),
  hooks: (lng) => __variableDynamicImportRuntimeHelper(/* @__PURE__ */ Object.assign({ "./locales/en/hooks.ts": () => __vitePreload(() => import("./hooks.js"), true ? [] : void 0), "./locales/fr/hooks.ts": () => __vitePreload(() => import("./hooks2.js"), true ? [] : void 0), "./locales/zh/hooks.ts": () => __vitePreload(() => import("./hooks3.js"), true ? [] : void 0) }), `./locales/${lng}/hooks.ts`, 4),
  projects: (lng) => __variableDynamicImportRuntimeHelper(/* @__PURE__ */ Object.assign({ "./locales/en/projects.ts": () => __vitePreload(() => import("./projects.js"), true ? [] : void 0), "./locales/fr/projects.ts": () => __vitePreload(() => import("./projects2.js"), true ? [] : void 0), "./locales/zh/projects.ts": () => __vitePreload(() => import("./projects3.js"), true ? [] : void 0) }), `./locales/${lng}/projects.ts`, 4),
  images: (lng) => __variableDynamicImportRuntimeHelper(/* @__PURE__ */ Object.assign({ "./locales/en/images.ts": () => __vitePreload(() => import("./images.js"), true ? [] : void 0), "./locales/fr/images.ts": () => __vitePreload(() => import("./images2.js"), true ? [] : void 0), "./locales/zh/images.ts": () => __vitePreload(() => import("./images3.js"), true ? [] : void 0) }), `./locales/${lng}/images.ts`, 4),
  metrics: (lng) => __variableDynamicImportRuntimeHelper(/* @__PURE__ */ Object.assign({ "./locales/en/metrics.ts": () => __vitePreload(() => import("./metrics.js"), true ? [] : void 0), "./locales/fr/metrics.ts": () => __vitePreload(() => import("./metrics2.js"), true ? [] : void 0), "./locales/zh/metrics.ts": () => __vitePreload(() => import("./metrics3.js"), true ? [] : void 0) }), `./locales/${lng}/metrics.ts`, 4),
  pwa: (lng) => __variableDynamicImportRuntimeHelper(/* @__PURE__ */ Object.assign({ "./locales/en/pwa.ts": () => __vitePreload(() => import("./pwa.js"), true ? [] : void 0), "./locales/fr/pwa.ts": () => __vitePreload(() => import("./pwa2.js"), true ? [] : void 0), "./locales/zh/pwa.ts": () => __vitePreload(() => import("./pwa3.js"), true ? [] : void 0) }), `./locales/${lng}/pwa.ts`, 4),
  sessions: (lng) => __variableDynamicImportRuntimeHelper(/* @__PURE__ */ Object.assign({ "./locales/en/sessions.ts": () => __vitePreload(() => import("./sessions.js"), true ? [] : void 0), "./locales/fr/sessions.ts": () => __vitePreload(() => import("./sessions2.js"), true ? [] : void 0), "./locales/zh/sessions.ts": () => __vitePreload(() => import("./sessions3.js"), true ? [] : void 0) }), `./locales/${lng}/sessions.ts`, 4),
  logs: (lng) => __variableDynamicImportRuntimeHelper(/* @__PURE__ */ Object.assign({ "./locales/en/logs.ts": () => __vitePreload(() => import("./logs.js"), true ? [] : void 0), "./locales/fr/logs.ts": () => __vitePreload(() => import("./logs2.js"), true ? [] : void 0), "./locales/zh/logs.ts": () => __vitePreload(() => import("./logs3.js"), true ? [] : void 0) }), `./locales/${lng}/logs.ts`, 4)
};
function loadLanguage(lng) {
  const keys = Object.keys(namespaces);
  const promises = keys.map(
    (ns) => namespaces[ns](lng).then((mod) => {
      instance.addResourceBundle(lng, ns, mod.default || mod, true, true);
    }).catch((err) => {
      console.warn(`[i18n] failed to load ${lng}/${ns}`, err);
    })
  );
  return Promise.all(promises);
}
function applyDocumentLocale(lng) {
  if (typeof document === "undefined" || !document.documentElement) return;
  document.documentElement.lang = lng || "en";
}
function init() {
  if (initPromise) return initPromise;
  initPromise = instance.init({
    lng: locale.value,
    fallbackLng: "en",
    defaultNS: "common",
    ns: Object.keys(namespaces),
    interpolation: {
      escapeValue: false
      // Preact / DOM handles escaping
    },
    resources: {}
  }).then(() => loadLanguage("en")).then(() => {
    if (locale.value !== "en") {
      return loadLanguage(locale.value);
    }
  }).then(() => {
    if (instance.language !== locale.value) {
      return void instance.changeLanguage(locale.value);
    }
  }).then(() => {
    applyDocumentLocale(locale.value);
  });
  return initPromise;
}
function t(key, opts) {
  return instance.t(key, opts);
}
function hasTranslation(key, opts) {
  return instance.exists(key, opts);
}
function useTranslation(ns) {
  const bound = useComputed(() => {
    locale.value;
    return {
      t: (key, opts) => {
        const options = opts ? Object.assign({ ns }, opts) : { ns };
        return instance.t(key, options);
      },
      locale: locale.value
    };
  });
  return bound.value;
}
function setLocale(lng) {
  const normalized = resolveSupportedLocale(lng);
  localStorage.setItem(STORAGE_KEY, normalized);
  return loadLanguage(normalized).then(
    () => instance.changeLanguage(normalized).then(() => {
      locale.value = normalized;
      applyDocumentLocale(normalized);
      translateStaticElements(document.documentElement);
      window.dispatchEvent(new CustomEvent("moltis:locale-changed", { detail: { locale: normalized } }));
    })
  );
}
function applyStaticTranslation(el, key, attrName) {
  if (!key) return;
  const translated = instance.t(key);
  if (!(translated && translated !== key)) return;
  if (attrName) {
    el.setAttribute(attrName, translated);
    return;
  }
  el.textContent = translated;
}
function translateStaticElements(root) {
  if (!root) return;
  const elements = root.querySelectorAll(
    "[data-i18n],[data-i18n-title],[data-i18n-placeholder],[data-i18n-aria-label]"
  );
  for (const el of elements) {
    applyStaticTranslation(el, el.getAttribute("data-i18n"));
    applyStaticTranslation(el, el.getAttribute("data-i18n-title"), "title");
    applyStaticTranslation(el, el.getAttribute("data-i18n-placeholder"), "placeholder");
    applyStaticTranslation(el, el.getAttribute("data-i18n-aria-label"), "aria-label");
  }
}
const _i18n = /* @__PURE__ */ Object.freeze(/* @__PURE__ */ Object.defineProperty({
  __proto__: null,
  getPreferredLocale,
  hasTranslation,
  init,
  locale,
  setLocale,
  supportedLocales,
  t,
  translateStaticElements,
  useTranslation
}, Symbol.toStringTag, { value: "Module" }));
const REASONING_SEP = "@reasoning-";
const models$1 = y$1([]);
const selectedModelId$1 = y$1(localStorage.getItem("moltis-model") || "");
const reasoningEffort = y$1(localStorage.getItem("moltis-reasoning-effort") || "");
const selectedModel = g$1(() => {
  const id = selectedModelId$1.value;
  return models$1.value.find((m2) => m2.id === id) || null;
});
const supportsReasoning = g$1(() => {
  const m2 = selectedModel.value;
  return !!(m2 == null ? void 0 : m2.supportsReasoning);
});
const effectiveModelId = g$1(() => {
  const id = selectedModelId$1.value;
  if (!id) return "";
  const effort = reasoningEffort.value;
  if (effort && supportsReasoning.value) return id + REASONING_SEP + effort;
  return id;
});
function parseReasoningSuffix(modelId) {
  if (!modelId) return { baseId: "", effort: "" };
  const idx = modelId.indexOf(REASONING_SEP);
  if (idx === -1) return { baseId: modelId, effort: "" };
  return { baseId: modelId.substring(0, idx), effort: modelId.substring(idx + REASONING_SEP.length) };
}
function isReasoningVariant(modelId) {
  return modelId.indexOf(REASONING_SEP) !== -1;
}
function setAll$2(arr) {
  models$1.value = arr || [];
}
function fetch$3() {
  return sendRpc("models.list", {}).then((r2) => {
    const res = r2;
    if (!(res == null ? void 0 : res.ok)) return;
    setAll$2(res.payload || []);
    if (models$1.value.length === 0) return;
    let saved = localStorage.getItem("moltis-model") || "";
    const parsed = parseReasoningSuffix(saved);
    if (parsed.effort) {
      saved = parsed.baseId;
      setReasoningEffort(parsed.effort);
      localStorage.setItem("moltis-model", saved);
    }
    const found = models$1.value.find((m2) => m2.id === saved);
    const model = found || models$1.value[0];
    select(model.id);
    if (!found) localStorage.setItem("moltis-model", model.id);
  });
}
function select(id) {
  selectedModelId$1.value = id;
}
function setReasoningEffort(effort) {
  reasoningEffort.value = effort || "";
  localStorage.setItem("moltis-reasoning-effort", effort || "");
}
function getById$1(id) {
  return models$1.value.find((m2) => m2.id === id) || null;
}
const modelStore = {
  models: models$1,
  selectedModelId: selectedModelId$1,
  selectedModel,
  reasoningEffort,
  supportsReasoning,
  effectiveModelId,
  parseReasoningSuffix,
  isReasoningVariant,
  setAll: setAll$2,
  fetch: fetch$3,
  select,
  setReasoningEffort,
  getById: getById$1
};
const _modelStore = /* @__PURE__ */ Object.freeze(/* @__PURE__ */ Object.defineProperty({
  __proto__: null,
  REASONING_SEP,
  effectiveModelId,
  fetch: fetch$3,
  getById: getById$1,
  isReasoningVariant,
  modelStore,
  models: models$1,
  parseReasoningSuffix,
  reasoningEffort,
  select,
  selectedModel,
  selectedModelId: selectedModelId$1,
  setAll: setAll$2,
  setReasoningEffort,
  supportsReasoning
}, Symbol.toStringTag, { value: "Module" }));
const projects$1 = y$1([]);
const activeProjectId$1 = y$1(localStorage.getItem("moltis-project") || "");
const projectFilterId$1 = y$1(localStorage.getItem("moltis-project-filter") || "");
function setAll$1(arr) {
  projects$1.value = arr || [];
}
function fetch$2() {
  return sendRpc("projects.list", {}).then((r2) => {
    const res = r2;
    if (!(res == null ? void 0 : res.ok)) return;
    setAll$1(res.payload || []);
  });
}
function setActiveProjectId$1(id) {
  activeProjectId$1.value = id || "";
}
function setFilterId(id) {
  projectFilterId$1.value = id || "";
  if (id) {
    localStorage.setItem("moltis-project-filter", id);
  } else {
    localStorage.removeItem("moltis-project-filter");
  }
}
function getById(id) {
  return projects$1.value.find((p2) => p2.id === id) || null;
}
const projectStore = {
  projects: projects$1,
  activeProjectId: activeProjectId$1,
  projectFilterId: projectFilterId$1,
  setAll: setAll$1,
  fetch: fetch$2,
  setActiveProjectId: setActiveProjectId$1,
  setFilterId,
  getById
};
const projectStore$1 = /* @__PURE__ */ Object.freeze(/* @__PURE__ */ Object.defineProperty({
  __proto__: null,
  activeProjectId: activeProjectId$1,
  fetch: fetch$2,
  getById,
  projectFilterId: projectFilterId$1,
  projectStore,
  projects: projects$1,
  setActiveProjectId: setActiveProjectId$1,
  setAll: setAll$1,
  setFilterId
}, Symbol.toStringTag, { value: "Module" }));
class Session {
  constructor(serverData) {
    // Server fields (plain properties, set on construction/update)
    __publicField(this, "key");
    __publicField(this, "label");
    __publicField(this, "model");
    __publicField(this, "provider");
    __publicField(this, "projectId");
    __publicField(this, "messageCount");
    __publicField(this, "lastSeenMessageCount");
    __publicField(this, "preview");
    __publicField(this, "updatedAt");
    __publicField(this, "createdAt");
    __publicField(this, "worktree_branch");
    __publicField(this, "sandbox_enabled");
    __publicField(this, "sandbox_image");
    __publicField(this, "channelBinding");
    __publicField(this, "parentSessionKey");
    __publicField(this, "forkPoint");
    __publicField(this, "agent_id");
    __publicField(this, "node_id");
    __publicField(this, "mcpDisabled");
    __publicField(this, "archived");
    __publicField(this, "activeChannel");
    __publicField(this, "version");
    // Client signals (reactive, per-session)
    __publicField(this, "replying");
    __publicField(this, "localUnread");
    __publicField(this, "streamText");
    __publicField(this, "voicePending");
    __publicField(this, "activeRunId");
    __publicField(this, "lastHistoryIndex");
    __publicField(this, "sessionTokens");
    __publicField(this, "contextWindow");
    __publicField(this, "toolsEnabled");
    __publicField(this, "lastToolOutput");
    __publicField(this, "badgeCount");
    __publicField(this, "dataVersion");
    this.key = serverData.key;
    this.label = serverData.label || "";
    this.model = serverData.model || "";
    this.provider = serverData.provider || "";
    this.projectId = serverData.projectId || "";
    this.messageCount = serverData.messageCount || 0;
    this.lastSeenMessageCount = serverData.lastSeenMessageCount || 0;
    this.preview = serverData.preview || "";
    this.updatedAt = serverData.updatedAt || 0;
    this.createdAt = serverData.createdAt || 0;
    this.worktree_branch = serverData.worktree_branch || "";
    this.sandbox_enabled = serverData.sandbox_enabled;
    this.sandbox_image = serverData.sandbox_image || null;
    this.channelBinding = serverData.channelBinding || null;
    this.parentSessionKey = serverData.parentSessionKey || "";
    this.forkPoint = serverData.forkPoint != null ? serverData.forkPoint : null;
    this.agent_id = serverData.agent_id || "main";
    this.node_id = serverData.node_id || null;
    this.mcpDisabled = serverData.mcpDisabled;
    this.archived = serverData.archived;
    this.activeChannel = serverData.activeChannel;
    this.version = serverData.version || 0;
    this.replying = y$1(false);
    this.localUnread = y$1(false);
    this.streamText = y$1("");
    this.voicePending = y$1(false);
    this.activeRunId = y$1(null);
    this.lastHistoryIndex = y$1(-1);
    this.sessionTokens = y$1({ input: 0, output: 0 });
    this.contextWindow = y$1(0);
    this.toolsEnabled = y$1(true);
    this.lastToolOutput = y$1("");
    this.badgeCount = y$1(this.messageCount);
    this.dataVersion = y$1(0);
  }
  /** Recalculate badge from current messageCount. */
  updateBadge() {
    this.badgeCount.value = this.messageCount;
  }
  /** Merge server fields, preserving client signals. Returns false if stale. */
  update(serverData) {
    const incoming = serverData.version || 0;
    if (incoming > 0 && this.version > 0 && incoming < this.version) return false;
    this.version = incoming || this.version;
    this.label = serverData.label || "";
    this.model = serverData.model || "";
    this.provider = serverData.provider || "";
    this.projectId = serverData.projectId || "";
    const serverCount = serverData.messageCount || 0;
    if (serverCount >= this.messageCount) {
      this.messageCount = serverCount;
      this.lastSeenMessageCount = serverData.lastSeenMessageCount || 0;
      this.preview = serverData.preview || "";
      this.updatedAt = serverData.updatedAt || 0;
    }
    this.createdAt = serverData.createdAt || 0;
    this.worktree_branch = serverData.worktree_branch || "";
    this.sandbox_enabled = serverData.sandbox_enabled;
    this.sandbox_image = serverData.sandbox_image || null;
    this.channelBinding = serverData.channelBinding || null;
    this.parentSessionKey = serverData.parentSessionKey || "";
    this.forkPoint = serverData.forkPoint != null ? serverData.forkPoint : null;
    this.agent_id = serverData.agent_id || "main";
    this.node_id = serverData.node_id || null;
    this.mcpDisabled = serverData.mcpDisabled;
    this.archived = serverData.archived;
    this.activeChannel = serverData.activeChannel;
    this.updateBadge();
    this.dataVersion.value++;
    return true;
  }
  /** Optimistic bump: increment total and mark seen if active. */
  bumpCount(increment) {
    this.messageCount = (this.messageCount || 0) + increment;
    if (this.key === activeSessionKey$1.value) {
      this.lastSeenMessageCount = this.messageCount;
    }
    this.updateBadge();
  }
  /** Authoritative set (switchSession history, /clear). */
  syncCounts(messageCount, lastSeenMessageCount) {
    this.messageCount = messageCount;
    this.lastSeenMessageCount = lastSeenMessageCount;
    this.updateBadge();
  }
  /** Clear streaming state for this session. */
  resetStreamState() {
    this.streamText.value = "";
    this.voicePending.value = false;
    this.activeRunId.value = null;
    this.lastToolOutput.value = "";
  }
  /** Return a plain SessionMeta snapshot of this session's server fields. */
  toMeta() {
    return {
      id: 0,
      key: this.key,
      label: this.label,
      model: this.model,
      provider: this.provider,
      createdAt: this.createdAt,
      updatedAt: this.updatedAt,
      messageCount: this.messageCount,
      lastSeenMessageCount: this.lastSeenMessageCount,
      projectId: this.projectId,
      sandbox_enabled: this.sandbox_enabled,
      sandbox_image: this.sandbox_image,
      worktree_branch: this.worktree_branch,
      channelBinding: this.channelBinding,
      activeChannel: this.activeChannel,
      parentSessionKey: this.parentSessionKey,
      forkPoint: this.forkPoint,
      mcpDisabled: this.mcpDisabled,
      preview: this.preview,
      archived: this.archived,
      agent_id: this.agent_id,
      node_id: this.node_id,
      version: this.version
    };
  }
}
const sessions$1 = y$1([]);
const activeSessionKey$1 = y$1(localStorage.getItem("moltis-session") || "main");
const switchInProgress = y$1(false);
const refreshInProgressKey = y$1("");
const sessionListTab = y$1(localStorage.getItem("moltis-session-tab") || "sessions");
const showArchivedSessions = y$1(localStorage.getItem("moltis-show-archived-sessions") === "1");
const activeSession = g$1(() => {
  const key = activeSessionKey$1.value;
  return sessions$1.value.find((s2) => s2.key === key) || null;
});
function compareSessionOrder(left, right) {
  const leftKey = (left == null ? void 0 : left.key) || "";
  const rightKey = (right == null ? void 0 : right.key) || "";
  const leftMain = leftKey === "main";
  const rightMain = rightKey === "main";
  if (leftMain !== rightMain) return leftMain ? -1 : 1;
  const updatedDiff = (Number(right == null ? void 0 : right.updatedAt) || 0) - (Number(left == null ? void 0 : left.updatedAt) || 0);
  if (updatedDiff !== 0) return updatedDiff;
  const createdDiff = (Number(right == null ? void 0 : right.createdAt) || 0) - (Number(left == null ? void 0 : left.createdAt) || 0);
  if (createdDiff !== 0) return createdDiff;
  return leftKey.localeCompare(rightKey);
}
function insertSessionInOrder(list, session) {
  if (!(session == null ? void 0 : session.key)) return Array.isArray(list) ? list.slice() : [];
  const result = Array.isArray(list) ? list.filter((entry) => (entry == null ? void 0 : entry.key) !== session.key) : [];
  result.push(session);
  result.sort(compareSessionOrder);
  return result;
}
function setAll(serverSessions) {
  const existing = {};
  for (const s2 of sessions$1.value) {
    existing[s2.key] = s2;
  }
  const result = [];
  for (const data of serverSessions) {
    const prev = existing[data.key];
    if (prev) {
      prev.update(data);
      if (data._localUnread) prev.localUnread.value = true;
      if (data._replying || data.replying) prev.replying.value = true;
      result.push(prev);
    } else {
      const session = new Session(data);
      if (data._localUnread) session.localUnread.value = true;
      if (data._replying || data.replying) session.replying.value = true;
      result.push(session);
    }
  }
  sessions$1.value = result;
}
function upsert(serverData) {
  if (!(serverData == null ? void 0 : serverData.key)) return null;
  const prev = getByKey(serverData.key);
  if (prev) {
    prev.update(serverData);
    sessions$1.value = insertSessionInOrder(sessions$1.value, prev);
    return prev;
  }
  const next = new Session(serverData);
  sessions$1.value = insertSessionInOrder(sessions$1.value, next);
  return next;
}
function remove(key) {
  var _a2, _b;
  if (!key) return false;
  const existing = getByKey(key);
  if (!existing) return false;
  sessions$1.value = sessions$1.value.filter((session) => session.key !== key);
  if (activeSessionKey$1.value === key) {
    const fallback = ((_a2 = sessions$1.value.find((session) => session.key === "main")) == null ? void 0 : _a2.key) || ((_b = sessions$1.value[0]) == null ? void 0 : _b.key) || "main";
    activeSessionKey$1.value = fallback;
    localStorage.setItem("moltis-session", fallback);
  }
  return true;
}
function fetch$1() {
  return window.fetch("/api/sessions", {
    headers: { Accept: "application/json" }
  }).then((response) => response.ok ? response.json() : null).then((payload) => {
    if (!Array.isArray(payload)) return;
    setAll(payload);
  }).catch(() => {
  });
}
function notify() {
  sessions$1.value = [...sessions$1.value];
}
function getByKey(key) {
  return sessions$1.value.find((s2) => s2.key === key) || null;
}
function setActive(key) {
  activeSessionKey$1.value = key;
  localStorage.setItem("moltis-session", key);
}
function setSessionListTab(tab) {
  sessionListTab.value = tab;
  localStorage.setItem("moltis-session-tab", tab);
}
function setShowArchivedSessions(show) {
  showArchivedSessions.value = !!show;
  localStorage.setItem("moltis-show-archived-sessions", show ? "1" : "0");
}
const sessionStore = {
  sessions: sessions$1,
  activeSessionKey: activeSessionKey$1,
  activeSession,
  switchInProgress,
  refreshInProgressKey,
  sessionListTab,
  showArchivedSessions,
  Session,
  setAll,
  upsert,
  remove,
  fetch: fetch$1,
  getByKey,
  setActive,
  setSessionListTab,
  setShowArchivedSessions,
  notify
};
const _sessionStoreModule = /* @__PURE__ */ Object.freeze(/* @__PURE__ */ Object.defineProperty({
  __proto__: null,
  Session,
  activeSession,
  activeSessionKey: activeSessionKey$1,
  compareSessionOrder,
  fetch: fetch$1,
  getByKey,
  insertSessionInOrder,
  notify,
  refreshInProgressKey,
  remove,
  sessionListTab,
  sessionStore,
  sessions: sessions$1,
  setActive,
  setAll,
  setSessionListTab,
  setShowArchivedSessions,
  showArchivedSessions,
  switchInProgress,
  upsert
}, Symbol.toStringTag, { value: "Module" }));
const connected$1 = y$1(false);
const cachedChannels$1 = y$1(null);
const unseenErrors$1 = y$1(0);
const unseenWarns$1 = y$1(0);
const sandboxInfo$1 = y$1(null);
let ws = null;
let reqId = 0;
let connected = false;
let subscribed = false;
let reconnectDelay = 1e3;
const pending = {};
let models = [];
let activeSessionKey = localStorage.getItem("moltis-session") || "main";
let activeProjectId = localStorage.getItem("moltis-project") || "";
let sessions = [];
let projects = [];
let streamEl = null;
let streamText = "";
let lastToolOutput = "";
let voicePending = false;
let chatHistory = JSON.parse(localStorage.getItem("moltis-chat-history") || "[]");
let chatHistoryIdx = -1;
let chatHistoryDraft = "";
let chatSeq = 0;
let sessionTokens = { input: 0, output: 0 };
let sessionCurrentInputTokens = 0;
let modelCombo = null;
let modelComboBtn = null;
let modelComboLabel = null;
let modelDropdown = null;
let modelSearchInput = null;
let modelDropdownList = null;
let selectedModelId = localStorage.getItem("moltis-model") || "";
let modelIdx = -1;
let nodeCombo = null;
let nodeComboBtn = null;
let nodeComboLabel = null;
let nodeDropdown = null;
let nodeDropdownList = null;
let projectCombo = null;
let projectComboBtn = null;
let projectComboLabel = null;
let projectDropdown = null;
let projectDropdownList = null;
let sandboxToggleBtn = null;
let sandboxLabel = null;
let sessionSandboxEnabled = true;
let sessionSandboxImage = null;
let sandboxImageBtn = null;
let sandboxImageDropdown = null;
let sandboxImageLabel = null;
let chatMsgBox = null;
let chatInput = null;
let chatSendBtn = null;
let chatBatchLoading = false;
let autoScrollMode = localStorage.getItem("moltis-auto-scroll") || "smart";
let sessionSwitchInProgress = false;
let lastHistoryIndex = -1;
let sessionContextWindow = 0;
let sessionToolsEnabled = true;
let sessionExecMode = "host";
let sessionExecPromptSymbol = "$";
let hostExecIsRoot = false;
let commandModeEnabled = false;
let refreshProvidersPage = null;
let refreshChannelsPage = null;
let channelEventUnsub = null;
let cachedChannels = null;
function setCachedChannels(v2) {
  cachedChannels = v2;
  cachedChannels$1.value = v2;
}
let sandboxInfo = null;
let logsEventHandler = null;
let networkAuditEventHandler = null;
let unseenErrors = 0;
let unseenWarns = 0;
let projectFilterId = localStorage.getItem("moltis-project-filter") || "";
function $(id) {
  return document.getElementById(id);
}
function setWs(v2) {
  ws = v2;
}
function setReqId(v2) {
  reqId = v2;
}
function setConnected(v2) {
  connected = v2;
  connected$1.value = v2;
}
function setSubscribed(v2) {
  subscribed = v2;
}
function setReconnectDelay(v2) {
  reconnectDelay = v2;
}
function setModels(v2) {
  models = v2;
}
function setActiveSessionKey(v2) {
  activeSessionKey = v2;
}
function setActiveProjectId(v2) {
  activeProjectId = v2;
}
function setSessions(v2) {
  sessions = v2;
}
function setProjects(v2) {
  projects = v2;
}
function setStreamEl(v2) {
  streamEl = v2;
}
function setStreamText(v2) {
  streamText = v2;
}
function setLastToolOutput(v2) {
  lastToolOutput = v2;
}
function setVoicePending(v2) {
  voicePending = v2;
}
function setChatHistory(v2) {
  chatHistory = v2;
}
function setChatHistoryIdx(v2) {
  chatHistoryIdx = v2;
}
function setChatHistoryDraft(v2) {
  chatHistoryDraft = v2;
}
function setChatSeq(v2) {
  chatSeq = v2;
}
function setSessionTokens(v2) {
  sessionTokens = v2;
}
function setSessionCurrentInputTokens(v2) {
  sessionCurrentInputTokens = v2;
}
function setModelCombo(v2) {
  modelCombo = v2;
}
function setModelComboBtn(v2) {
  modelComboBtn = v2;
}
function setModelComboLabel(v2) {
  modelComboLabel = v2;
}
function setModelDropdown(v2) {
  modelDropdown = v2;
}
function setModelSearchInput(v2) {
  modelSearchInput = v2;
}
function setModelDropdownList(v2) {
  modelDropdownList = v2;
}
function setSelectedModelId(v2) {
  selectedModelId = v2;
}
function setModelIdx(v2) {
  modelIdx = v2;
}
function setNodeCombo(v2) {
  nodeCombo = v2;
}
function setNodeComboBtn(v2) {
  nodeComboBtn = v2;
}
function setNodeComboLabel(v2) {
  nodeComboLabel = v2;
}
function setNodeDropdown(v2) {
  nodeDropdown = v2;
}
function setNodeDropdownList(v2) {
  nodeDropdownList = v2;
}
function setProjectCombo(v2) {
  projectCombo = v2;
}
function setProjectComboBtn(v2) {
  projectComboBtn = v2;
}
function setProjectComboLabel(v2) {
  projectComboLabel = v2;
}
function setProjectDropdown(v2) {
  projectDropdown = v2;
}
function setProjectDropdownList(v2) {
  projectDropdownList = v2;
}
function setSandboxToggleBtn(v2) {
  sandboxToggleBtn = v2;
}
function setSandboxLabel(v2) {
  sandboxLabel = v2;
}
function setSessionSandboxEnabled(v2) {
  sessionSandboxEnabled = v2;
}
function setSessionSandboxImage(v2) {
  sessionSandboxImage = v2;
}
function setSandboxImageBtn(v2) {
  sandboxImageBtn = v2;
}
function setSandboxImageDropdown(v2) {
  sandboxImageDropdown = v2;
}
function setSandboxImageLabel(v2) {
  sandboxImageLabel = v2;
}
function setChatMsgBox(v2) {
  chatMsgBox = v2;
}
function setChatInput(v2) {
  chatInput = v2;
}
function setChatSendBtn(v2) {
  chatSendBtn = v2;
}
function setChatBatchLoading(v2) {
  chatBatchLoading = v2;
}
function setAutoScrollMode(v2) {
  autoScrollMode = v2;
  localStorage.setItem("moltis-auto-scroll", v2);
}
function setSessionSwitchInProgress(v2) {
  sessionSwitchInProgress = v2;
}
function setLastHistoryIndex(v2) {
  lastHistoryIndex = v2;
}
function setSessionContextWindow(v2) {
  sessionContextWindow = v2;
}
function setSessionToolsEnabled(v2) {
  sessionToolsEnabled = v2;
}
function setSessionExecMode(v2) {
  sessionExecMode = v2;
}
function setSessionExecPromptSymbol(v2) {
  sessionExecPromptSymbol = v2;
}
function setHostExecIsRoot(v2) {
  hostExecIsRoot = !!v2;
}
function setCommandModeEnabled(v2) {
  commandModeEnabled = !!v2;
}
function setRefreshProvidersPage(v2) {
  refreshProvidersPage = v2;
}
function setRefreshChannelsPage(v2) {
  refreshChannelsPage = v2;
}
function setChannelEventUnsub(v2) {
  channelEventUnsub = v2;
}
function setLogsEventHandler(v2) {
  logsEventHandler = v2;
}
function setNetworkAuditEventHandler(v2) {
  networkAuditEventHandler = v2;
}
function setUnseenErrors(v2) {
  unseenErrors = v2;
  unseenErrors$1.value = v2;
}
function setUnseenWarns(v2) {
  unseenWarns = v2;
  unseenWarns$1.value = v2;
}
function setProjectFilterId(v2) {
  projectFilterId = v2;
}
function setSandboxInfo(v2) {
  sandboxInfo = v2;
  sandboxInfo$1.value = v2;
}
const S = /* @__PURE__ */ Object.freeze(/* @__PURE__ */ Object.defineProperty({
  __proto__: null,
  $,
  get activeProjectId() {
    return activeProjectId;
  },
  get activeSessionKey() {
    return activeSessionKey;
  },
  get autoScrollMode() {
    return autoScrollMode;
  },
  get cachedChannels() {
    return cachedChannels;
  },
  get channelEventUnsub() {
    return channelEventUnsub;
  },
  get chatBatchLoading() {
    return chatBatchLoading;
  },
  get chatHistory() {
    return chatHistory;
  },
  get chatHistoryDraft() {
    return chatHistoryDraft;
  },
  get chatHistoryIdx() {
    return chatHistoryIdx;
  },
  get chatInput() {
    return chatInput;
  },
  get chatMsgBox() {
    return chatMsgBox;
  },
  get chatSendBtn() {
    return chatSendBtn;
  },
  get chatSeq() {
    return chatSeq;
  },
  get commandModeEnabled() {
    return commandModeEnabled;
  },
  get connected() {
    return connected;
  },
  get hostExecIsRoot() {
    return hostExecIsRoot;
  },
  get lastHistoryIndex() {
    return lastHistoryIndex;
  },
  get lastToolOutput() {
    return lastToolOutput;
  },
  get logsEventHandler() {
    return logsEventHandler;
  },
  get modelCombo() {
    return modelCombo;
  },
  get modelComboBtn() {
    return modelComboBtn;
  },
  get modelComboLabel() {
    return modelComboLabel;
  },
  get modelDropdown() {
    return modelDropdown;
  },
  get modelDropdownList() {
    return modelDropdownList;
  },
  get modelIdx() {
    return modelIdx;
  },
  get modelSearchInput() {
    return modelSearchInput;
  },
  get models() {
    return models;
  },
  get networkAuditEventHandler() {
    return networkAuditEventHandler;
  },
  get nodeCombo() {
    return nodeCombo;
  },
  get nodeComboBtn() {
    return nodeComboBtn;
  },
  get nodeComboLabel() {
    return nodeComboLabel;
  },
  get nodeDropdown() {
    return nodeDropdown;
  },
  get nodeDropdownList() {
    return nodeDropdownList;
  },
  pending,
  get projectCombo() {
    return projectCombo;
  },
  get projectComboBtn() {
    return projectComboBtn;
  },
  get projectComboLabel() {
    return projectComboLabel;
  },
  get projectDropdown() {
    return projectDropdown;
  },
  get projectDropdownList() {
    return projectDropdownList;
  },
  get projectFilterId() {
    return projectFilterId;
  },
  get projects() {
    return projects;
  },
  get reconnectDelay() {
    return reconnectDelay;
  },
  get refreshChannelsPage() {
    return refreshChannelsPage;
  },
  get refreshProvidersPage() {
    return refreshProvidersPage;
  },
  get reqId() {
    return reqId;
  },
  get sandboxImageBtn() {
    return sandboxImageBtn;
  },
  get sandboxImageDropdown() {
    return sandboxImageDropdown;
  },
  get sandboxImageLabel() {
    return sandboxImageLabel;
  },
  get sandboxInfo() {
    return sandboxInfo;
  },
  get sandboxLabel() {
    return sandboxLabel;
  },
  get sandboxToggleBtn() {
    return sandboxToggleBtn;
  },
  get selectedModelId() {
    return selectedModelId;
  },
  get sessionContextWindow() {
    return sessionContextWindow;
  },
  get sessionCurrentInputTokens() {
    return sessionCurrentInputTokens;
  },
  get sessionExecMode() {
    return sessionExecMode;
  },
  get sessionExecPromptSymbol() {
    return sessionExecPromptSymbol;
  },
  get sessionSandboxEnabled() {
    return sessionSandboxEnabled;
  },
  get sessionSandboxImage() {
    return sessionSandboxImage;
  },
  get sessionSwitchInProgress() {
    return sessionSwitchInProgress;
  },
  get sessionTokens() {
    return sessionTokens;
  },
  get sessionToolsEnabled() {
    return sessionToolsEnabled;
  },
  get sessions() {
    return sessions;
  },
  setActiveProjectId,
  setActiveSessionKey,
  setAutoScrollMode,
  setCachedChannels,
  setChannelEventUnsub,
  setChatBatchLoading,
  setChatHistory,
  setChatHistoryDraft,
  setChatHistoryIdx,
  setChatInput,
  setChatMsgBox,
  setChatSendBtn,
  setChatSeq,
  setCommandModeEnabled,
  setConnected,
  setHostExecIsRoot,
  setLastHistoryIndex,
  setLastToolOutput,
  setLogsEventHandler,
  setModelCombo,
  setModelComboBtn,
  setModelComboLabel,
  setModelDropdown,
  setModelDropdownList,
  setModelIdx,
  setModelSearchInput,
  setModels,
  setNetworkAuditEventHandler,
  setNodeCombo,
  setNodeComboBtn,
  setNodeComboLabel,
  setNodeDropdown,
  setNodeDropdownList,
  setProjectCombo,
  setProjectComboBtn,
  setProjectComboLabel,
  setProjectDropdown,
  setProjectDropdownList,
  setProjectFilterId,
  setProjects,
  setReconnectDelay,
  setRefreshChannelsPage,
  setRefreshProvidersPage,
  setReqId,
  setSandboxImageBtn,
  setSandboxImageDropdown,
  setSandboxImageLabel,
  setSandboxInfo,
  setSandboxLabel,
  setSandboxToggleBtn,
  setSelectedModelId,
  setSessionContextWindow,
  setSessionCurrentInputTokens,
  setSessionExecMode,
  setSessionExecPromptSymbol,
  setSessionSandboxEnabled,
  setSessionSandboxImage,
  setSessionSwitchInProgress,
  setSessionTokens,
  setSessionToolsEnabled,
  setSessions,
  setStreamEl,
  setStreamText,
  setSubscribed,
  setUnseenErrors,
  setUnseenWarns,
  setVoicePending,
  setWs,
  get streamEl() {
    return streamEl;
  },
  get streamText() {
    return streamText;
  },
  get subscribed() {
    return subscribed;
  },
  get unseenErrors() {
    return unseenErrors;
  },
  get unseenWarns() {
    return unseenWarns;
  },
  get voicePending() {
    return voicePending;
  },
  get ws() {
    return ws;
  }
}, Symbol.toStringTag, { value: "Module" }));
function modelVersionScore(id) {
  const matches = (id || "").match(/\d+(?:\.\d+)?/g);
  if (!matches) return 0;
  let max = 0;
  for (const m2 of matches) {
    const v2 = Number.parseFloat(m2);
    if (v2 > max) max = v2;
  }
  return max;
}
function translatedOrFallback(key, opts, fallback) {
  if (!key) return fallback;
  if (!hasTranslation(key, opts)) return fallback;
  const translated = t(key, opts);
  if (translated) return translated;
  return fallback;
}
function nextId() {
  setReqId(reqId + 1);
  return `ui-${reqId}`;
}
function esc(s2) {
  return s2.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}
function stripAnsi(text) {
  const input = String(text || "");
  let out = "";
  for (let i2 = 0; i2 < input.length; i2++) {
    if (input.charCodeAt(i2) === 27 && input[i2 + 1] === "[") {
      i2 += 2;
      while (i2 < input.length) {
        const ch = input[i2];
        if (ch >= "@" && ch <= "~") break;
        i2++;
      }
      continue;
    }
    out += input[i2];
  }
  return out;
}
function splitPipeCells(line) {
  let plain = stripAnsi(line).trim();
  if (plain.startsWith("|")) plain = plain.slice(1);
  if (plain.endsWith("|")) plain = plain.slice(0, -1);
  return plain.split("|").map((cell) => cell.trim());
}
function normalizeTableRow(cells, columnCount) {
  const row = cells.slice(0, columnCount);
  while (row.length < columnCount) row.push("");
  return row;
}
function buildTableHtml(headerCells, bodyRows) {
  const columnCount = headerCells.length;
  const headerRow = normalizeTableRow(headerCells, columnCount);
  const bodyHtml = bodyRows.map((row) => normalizeTableRow(row, columnCount)).map((row) => `<tr>${row.map((cell) => `<td>${cell}</td>`).join("")}</tr>`).join("");
  const thead = `<thead><tr>${headerRow.map((cell) => `<th>${cell}</th>`).join("")}</tr></thead>`;
  const tbody = bodyRows.length > 0 ? `<tbody>${bodyHtml}</tbody>` : "";
  return `<div class="msg-table-wrap"><table class="msg-table">${thead}${tbody}</table></div>`;
}
function isAsciiBorderRow(line) {
  return /^\+(?:[-=]+\+)+$/.test(stripAnsi(line).trim());
}
function isAsciiPipeRow(line) {
  return /^\|.*\|$/.test(stripAnsi(line).trim());
}
function parseAsciiTable(lines, start) {
  if (!isAsciiBorderRow(lines[start])) return null;
  let next = start + 1;
  const rows = [];
  while (next < lines.length) {
    const line = lines[next];
    if (isAsciiBorderRow(line)) {
      next++;
      continue;
    }
    if (!isAsciiPipeRow(line)) break;
    rows.push(splitPipeCells(line));
    next++;
  }
  if (rows.length === 0) return null;
  return {
    html: buildTableHtml(rows[0], rows.slice(1)),
    next
  };
}
function extractAsciiTables(s2) {
  const lines = s2.split("\n");
  const out = [];
  const tables = [];
  for (let i2 = 0; i2 < lines.length; ) {
    const asciiTable = parseAsciiTable(lines, i2);
    if (asciiTable) {
      out.push(`@@MOLTIS_ASCII_TABLE_${tables.length}@@`);
      tables.push(asciiTable.html);
      i2 = asciiTable.next;
      continue;
    }
    out.push(lines[i2]);
    i2++;
  }
  return { text: out.join("\n"), tables };
}
function sanitizeHref(href) {
  const trimmed = href.trim();
  if (/^(https?:|mailto:|#)/i.test(trimmed)) return trimmed;
  return "#";
}
const mdRenderer = new y$3();
mdRenderer.code = ({ text, lang }) => {
  const langAttr = lang ? ` data-lang="${esc(lang)}"` : "";
  const badge = lang ? `<div class="code-lang-badge">${esc(lang)}</div>` : "";
  return `<pre class="code-block">${badge}<code${langAttr}>${esc(text)}</code></pre>
`;
};
mdRenderer.table = function(token) {
  const renderCell = (cell) => this.parser.parseInline(cell.tokens);
  const header = token.header.map((cell) => `<th>${renderCell(cell)}</th>`).join("");
  const body = token.rows.map(
    (row) => `<tr>${row.map((cell) => `<td>${renderCell(cell)}</td>`).join("")}</tr>`
  ).join("");
  return `<div class="msg-table-wrap"><table class="msg-table"><thead><tr>${header}</tr></thead><tbody>${body}</tbody></table></div>
`;
};
mdRenderer.link = ({ href, text }) => {
  return `<a href="${sanitizeHref(href)}" target="_blank" rel="noopener noreferrer">${text}</a>`;
};
mdRenderer.html = ({ text }) => esc(text);
const markedInstance = new D$1({ renderer: mdRenderer, breaks: true, gfm: true, async: false });
function renderMarkdown(raw) {
  const { text, tables } = extractAsciiTables(raw);
  let result = markedInstance.parse(text);
  result = result.replace(/@@MOLTIS_ASCII_TABLE_(\d+)@@/g, (_2, idx) => tables[Number(idx)] || "");
  return result;
}
function sendRpc(method, params) {
  return new Promise((resolve) => {
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      resolve({
        ok: false,
        error: {
          code: "UNAVAILABLE",
          message: localizedRpcErrorMessage({
            code: "UNAVAILABLE",
            message: "WebSocket not connected"
          })
        }
      });
      return;
    }
    const id = nextId();
    pending[id] = resolve;
    ws.send(JSON.stringify({ type: "req", id, method, params }));
  });
}
function localizedRpcErrorMessage(error) {
  if (!error) return t("errors:generic.title");
  if (error.code) {
    const key = `errors:codes.${error.code}`;
    const translated = t(key);
    if (translated && translated !== key) {
      return translated;
    }
  }
  return error.message || t("errors:generic.title");
}
function localizedApiErrorMessage(payload, fallbackMessage) {
  if (payload && typeof payload.error === "object") {
    return localizedRpcErrorMessage(payload.error);
  }
  if (payload == null ? void 0 : payload.code) {
    const key = `errors:codes.${payload.code}`;
    const translated = t(key);
    if (translated && translated !== key) {
      return translated;
    }
  }
  if (payload && typeof payload.error === "string" && payload.error.trim()) {
    return payload.error;
  }
  if (payload && typeof payload.message === "string" && payload.message.trim()) {
    return payload.message;
  }
  return fallbackMessage || t("errors:generic.title");
}
function localizeRpcError(error) {
  if (!error) return error;
  const message = localizedRpcErrorMessage(error);
  if (error.message === message) return error;
  return Object.assign({}, error, { message, serverMessage: error.message });
}
function localizeStructuredError(error) {
  if (!error) return error;
  const title = translatedOrFallback(error.title_key, error.title_params, error.title || t("errors:generic.title"));
  const detail = translatedOrFallback(error.detail_key, error.detail_params, error.detail || "");
  if (title === error.title && detail === (error.detail || "")) return error;
  return Object.assign({}, error, { title, detail });
}
function formatTokens(n2) {
  if (n2 >= 1e6) return `${(n2 / 1e6).toFixed(1)}M`;
  if (n2 >= 1e3) return `${(n2 / 1e3).toFixed(1)}K`;
  return String(n2);
}
function formatAssistantTokenUsage(inputTokens, outputTokens, cacheReadTokens) {
  const input = Number(inputTokens || 0);
  const output = Number(outputTokens || 0);
  const cached = Number(cacheReadTokens || 0);
  let inputText = `${formatTokens(input)} in`;
  if (cached > 0) {
    inputText += ` (${formatTokens(cached)} cached)`;
  }
  return `${inputText} / ${formatTokens(output)} out`;
}
const TOKEN_SPEED_SLOW_TPS = 10;
const TOKEN_SPEED_FAST_TPS = 25;
function tokenSpeedPerSecond(outputTokens, durationMs) {
  const out = Number(outputTokens || 0);
  const ms = Number(durationMs || 0);
  if (!(out > 0 && ms > 0)) return null;
  const speed = out * 1e3 / ms;
  return Number.isFinite(speed) && speed > 0 ? speed : null;
}
function formatTokenSpeed(outputTokens, durationMs) {
  const speed = tokenSpeedPerSecond(outputTokens, durationMs);
  if (speed == null) return null;
  if (speed >= 100) return `${speed.toFixed(0)} tok/s`;
  if (speed >= 10) return `${speed.toFixed(1)} tok/s`;
  return `${speed.toFixed(2)} tok/s`;
}
function tokenSpeedTone(outputTokens, durationMs) {
  const speed = tokenSpeedPerSecond(outputTokens, durationMs);
  if (speed == null) return null;
  if (speed < TOKEN_SPEED_SLOW_TPS) return "slow";
  if (speed >= TOKEN_SPEED_FAST_TPS) return "fast";
  return "normal";
}
function formatBytes(b2) {
  if (b2 >= 1024) return `${(b2 / 1024).toFixed(1)} KB`;
  return `${b2} B`;
}
function getResetsAtMs(errObj) {
  return errObj.resetsAt || (errObj.resets_at ? errObj.resets_at * 1e3 : null);
}
function classifyStructuredError(errObj, resetsAt) {
  if (!(errObj.title_key || errObj.detail_key)) return null;
  const result = localizeStructuredError({
    icon: errObj.icon || "⚠️",
    title: errObj.title || t("errors:generic.title"),
    detail: errObj.detail || errObj.message || "",
    provider: errObj.provider,
    resetsAt,
    title_key: errObj.title_key,
    detail_key: errObj.detail_key,
    title_params: errObj.title_params,
    detail_params: errObj.detail_params
  });
  if (!result) return null;
  return {
    icon: result.icon || "⚠️",
    title: result.title || t("errors:generic.title"),
    detail: result.detail || "",
    provider: result.provider,
    resetsAt: result.resetsAt ?? null
  };
}
function classifyUsageLimitError(errObj, resetsAt) {
  if (!(errObj.type === "usage_limit_reached" || errObj.message && String(errObj.message).indexOf("usage limit") !== -1)) {
    return null;
  }
  return {
    icon: "",
    title: t("errors:usageLimitReached.title"),
    detail: t("errors:usageLimitReached.detail", { planType: errObj.plan_type || "current" }),
    resetsAt
  };
}
function classifyRateLimitError(errObj, resetsAt) {
  if (!(errObj.type === "rate_limit_exceeded" || errObj.message && String(errObj.message).indexOf("rate limit") !== -1)) {
    return null;
  }
  return {
    icon: "⚠️",
    title: t("errors:rateLimited.title"),
    detail: errObj.message || t("errors:rateLimited.detail"),
    resetsAt
  };
}
function classifyJsonErrorObj(errObj) {
  const resetsAt = getResetsAtMs(errObj);
  return classifyStructuredError(errObj, resetsAt) || classifyUsageLimitError(errObj, resetsAt) || classifyRateLimitError(errObj, resetsAt) || (errObj.message ? { icon: "⚠️", title: t("errors:generic.title"), detail: errObj.message, resetsAt: null } : null);
}
function parseJsonError(message) {
  const jsonMatch = message.match(/\{[\s\S]*\}$/);
  if (!jsonMatch) return null;
  try {
    const err = JSON.parse(jsonMatch[0]);
    return classifyJsonErrorObj(err.error || err);
  } catch (_e2) {
  }
  return null;
}
function parseHttpStatusError(message) {
  const statusMatch = message.match(/HTTP (\d{3})/);
  const code = statusMatch ? parseInt(statusMatch[1], 10) : 0;
  if (code === 401 || code === 403)
    return {
      icon: "🔒",
      title: t("errors:authError.title"),
      detail: t("errors:authError.detail"),
      resetsAt: null
    };
  if (code === 429)
    return {
      icon: "",
      title: t("errors:rateLimited.title"),
      detail: t("errors:rateLimited.detailShort"),
      resetsAt: null
    };
  if (code >= 500)
    return {
      icon: "🚨",
      title: t("errors:serverError.title"),
      detail: t("errors:serverError.detail"),
      resetsAt: null
    };
  return null;
}
function parseErrorMessage(message) {
  return parseJsonError(message) || parseHttpStatusError(message) || {
    icon: "⚠️",
    title: t("errors:generic.title"),
    detail: message,
    resetsAt: null
  };
}
function updateCountdown(el, resetsAtMs) {
  const now = Date.now();
  const diff = resetsAtMs - now;
  if (diff <= 0) {
    el.textContent = t("errors:countdown.resetReady");
    el.className = "error-countdown reset-ready";
    return true;
  }
  const hours = Math.floor(diff / 36e5);
  const mins = Math.floor(diff % 36e5 / 6e4);
  const parts = [];
  if (hours > 0) parts.push(`${hours}h`);
  parts.push(`${mins}m`);
  el.textContent = t("errors:countdown.resetsIn", { time: parts.join(" ") });
  return false;
}
function toolCallSummary(name, args, executionMode) {
  if (!args) return name || "tool";
  switch (name) {
    case "exec": {
      const command = args.command || "exec";
      const nodeRef = typeof args.node === "string" ? args.node.trim() : "";
      if (!nodeRef) return command;
      if (nodeRef.startsWith("ssh:target:")) {
        return `${command} [SSH target]`;
      }
      if (nodeRef.startsWith("ssh:")) {
        return `${command} [SSH: ${nodeRef.slice(4)}]`;
      }
      if (nodeRef.includes("@")) {
        return `${command} [SSH: ${nodeRef}]`;
      }
      return `${command} [node: ${nodeRef}]`;
    }
    case "web_fetch":
      return `web_fetch ${args.url || ""}`.trim();
    case "web_search":
      return `web_search "${args.query || ""}"`;
    case "browser": {
      const action = args.action || "browser";
      const mode = executionMode ? ` (${executionMode})` : "";
      const url = args.url ? ` ${args.url}` : "";
      return `browser ${action}${mode}${url}`.trim();
    }
    default:
      return name || "tool";
  }
}
function renderScreenshot(container, imgSrc, scale) {
  if (!scale) scale = 1;
  const imgContainer = document.createElement("div");
  imgContainer.className = "screenshot-container";
  const img = document.createElement("img");
  img.src = imgSrc;
  img.className = "screenshot-thumbnail";
  img.alt = "Browser screenshot";
  img.title = "Click to view full size";
  const effectiveScale = scale;
  img.onload = () => {
    if (effectiveScale > 1) {
      const logicalWidth = img.naturalWidth / effectiveScale;
      const logicalHeight = img.naturalHeight / effectiveScale;
      img.style.aspectRatio = `${logicalWidth} / ${logicalHeight}`;
    }
  };
  const downloadScreenshot = (e2) => {
    e2.stopPropagation();
    const link = document.createElement("a");
    link.href = imgSrc;
    link.download = `screenshot-${Date.now()}.png`;
    link.click();
  };
  img.onclick = () => {
    const overlay = document.createElement("div");
    overlay.className = "screenshot-lightbox";
    const lightboxContent = document.createElement("div");
    lightboxContent.className = "screenshot-lightbox-content";
    const header = document.createElement("div");
    header.className = "screenshot-lightbox-header";
    header.onclick = (e2) => e2.stopPropagation();
    const closeBtn = document.createElement("button");
    closeBtn.className = "screenshot-lightbox-close";
    closeBtn.textContent = "✕";
    closeBtn.title = "Close (Esc)";
    closeBtn.onclick = () => overlay.remove();
    const downloadBtn = document.createElement("button");
    downloadBtn.className = "screenshot-download-btn";
    downloadBtn.textContent = "⬇ Download";
    downloadBtn.onclick = downloadScreenshot;
    header.appendChild(closeBtn);
    header.appendChild(downloadBtn);
    const scrollContainer = document.createElement("div");
    scrollContainer.className = "screenshot-lightbox-scroll";
    scrollContainer.onclick = (e2) => e2.stopPropagation();
    const fullImg = document.createElement("img");
    fullImg.src = img.src;
    fullImg.className = "screenshot-lightbox-img";
    fullImg.onload = () => {
      const logicalWidth = fullImg.naturalWidth / effectiveScale;
      const logicalHeight = fullImg.naturalHeight / effectiveScale;
      const viewportWidth = window.innerWidth - 80;
      const displayWidth = Math.min(logicalWidth, viewportWidth);
      fullImg.style.width = `${displayWidth}px`;
      const displayHeight = displayWidth / logicalWidth * logicalHeight;
      fullImg.style.height = `${displayHeight}px`;
    };
    scrollContainer.appendChild(fullImg);
    lightboxContent.appendChild(header);
    lightboxContent.appendChild(scrollContainer);
    overlay.appendChild(lightboxContent);
    overlay.onclick = () => overlay.remove();
    const closeOnEscape = (e2) => {
      if (e2.key === "Escape") {
        overlay.remove();
        document.removeEventListener("keydown", closeOnEscape);
      }
    };
    document.addEventListener("keydown", closeOnEscape);
    document.body.appendChild(overlay);
  };
  const thumbDownloadBtn = document.createElement("button");
  thumbDownloadBtn.className = "screenshot-download-btn-small";
  thumbDownloadBtn.textContent = "⬇";
  thumbDownloadBtn.title = "Download screenshot";
  thumbDownloadBtn.onclick = downloadScreenshot;
  imgContainer.appendChild(img);
  imgContainer.appendChild(thumbDownloadBtn);
  container.appendChild(imgContainer);
}
function documentIcon(mimeType, filename) {
  var _a2;
  const ext = ((_a2 = (filename || "").split(".").pop()) == null ? void 0 : _a2.toLowerCase()) || "";
  if (mimeType === "application/pdf" || ext === "pdf") return "📄";
  if (mimeType === "application/zip" || mimeType === "application/gzip" || ext === "zip" || ext === "gz")
    return "📦";
  if (/spreadsheet|csv|xls/.test(mimeType || "") || /^(csv|xls|xlsx)$/.test(ext)) return "📊";
  if (/wordprocessing|msword|rtf/.test(mimeType || "") || /^(doc|docx|rtf)$/.test(ext)) return "📃";
  if (/presentation|ppt/.test(mimeType || "") || /^(ppt|pptx)$/.test(ext)) return "📊";
  return "📁";
}
function formatDocSize(bytes) {
  if (typeof bytes !== "number" || bytes < 0) return "";
  if (bytes >= 1048576) return `${(bytes / 1048576).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${bytes} B`;
}
function renderDocument(container, mediaSrc, filename, mimeType, sizeBytes) {
  const wrap = document.createElement("div");
  wrap.className = "document-container";
  const icon = document.createElement("span");
  icon.className = "document-icon";
  icon.textContent = documentIcon(mimeType, filename);
  const info = document.createElement("div");
  info.className = "document-info";
  const nameEl = document.createElement("span");
  nameEl.className = "document-filename";
  nameEl.textContent = filename || "document";
  info.appendChild(nameEl);
  if (sizeBytes != null && sizeBytes > 0) {
    const sizeEl = document.createElement("span");
    sizeEl.className = "document-size";
    sizeEl.textContent = formatDocSize(sizeBytes);
    info.appendChild(sizeEl);
  }
  const dlBtn = document.createElement("a");
  dlBtn.className = "document-download-btn";
  dlBtn.href = mediaSrc;
  dlBtn.download = filename || "document";
  const isPdf = (mimeType || "").includes("pdf") || (filename || "").endsWith(".pdf");
  const isText = (mimeType || "").startsWith("text/");
  if (isPdf || isText) {
    dlBtn.target = "_blank";
    dlBtn.rel = "noopener noreferrer";
    dlBtn.textContent = "↗ Open";
    dlBtn.removeAttribute("download");
  } else {
    dlBtn.textContent = "⬇ Download";
  }
  wrap.appendChild(icon);
  wrap.appendChild(info);
  wrap.appendChild(dlBtn);
  container.appendChild(wrap);
}
const WAVEFORM_BAR_COUNT = 48;
const WAVEFORM_MIN_HEIGHT = 0.08;
async function extractWaveform(audioSrc, barCount) {
  const ctx = new (window.AudioContext || window.webkitAudioContext)();
  try {
    const response = await fetch(audioSrc);
    const buf = await response.arrayBuffer();
    const audioBuffer = await ctx.decodeAudioData(buf);
    const data = audioBuffer.getChannelData(0);
    if (data.length < barCount) {
      return new Array(barCount).fill(WAVEFORM_MIN_HEIGHT);
    }
    const step = Math.floor(data.length / barCount);
    const peaks = [];
    for (let i2 = 0; i2 < barCount; i2++) {
      const start = i2 * step;
      const end = Math.min(start + step, data.length);
      let max = 0;
      for (let j2 = start; j2 < end; j2++) {
        const abs = Math.abs(data[j2]);
        if (abs > max) max = abs;
      }
      peaks.push(max);
    }
    let maxPeak = 0;
    for (const pk of peaks) {
      if (pk > maxPeak) maxPeak = pk;
    }
    maxPeak = maxPeak || 1;
    return peaks.map((v2) => Math.max(WAVEFORM_MIN_HEIGHT, v2 / maxPeak));
  } finally {
    ctx.close();
  }
}
function formatAudioDuration(seconds) {
  if (!Number.isFinite(seconds) || seconds < 0) return "00:00";
  const totalSeconds = Math.floor(seconds);
  const m2 = Math.floor(totalSeconds / 60);
  const s2 = totalSeconds % 60;
  return `${String(m2).padStart(2, "0")}:${String(s2).padStart(2, "0")}`;
}
function createPlaySvg() {
  const NS = "http://www.w3.org/2000/svg";
  const el = document.createElementNS(NS, "svg");
  el.setAttribute("viewBox", "0 0 24 24");
  el.setAttribute("aria-hidden", "true");
  el.setAttribute("focusable", "false");
  el.setAttribute("fill", "currentColor");
  el.setAttribute("preserveAspectRatio", "xMidYMid meet");
  const path = document.createElementNS(NS, "path");
  path.setAttribute("d", "M8 5v14l11-7z");
  el.appendChild(path);
  return el;
}
function createPauseSvg() {
  const NS = "http://www.w3.org/2000/svg";
  const el = document.createElementNS(NS, "svg");
  el.setAttribute("viewBox", "0 0 24 24");
  el.setAttribute("aria-hidden", "true");
  el.setAttribute("focusable", "false");
  el.setAttribute("fill", "currentColor");
  el.setAttribute("preserveAspectRatio", "xMidYMid meet");
  const left = document.createElementNS(NS, "rect");
  left.setAttribute("x", "6");
  left.setAttribute("y", "4");
  left.setAttribute("width", "4");
  left.setAttribute("height", "16");
  left.setAttribute("rx", "1");
  el.appendChild(left);
  const right = document.createElementNS(NS, "rect");
  right.setAttribute("x", "14");
  right.setAttribute("y", "4");
  right.setAttribute("width", "4");
  right.setAttribute("height", "16");
  right.setAttribute("rx", "1");
  el.appendChild(right);
  return el;
}
let _audioCtx = null;
function warmAudioPlayback() {
  if (!_audioCtx) {
    _audioCtx = new (window.AudioContext || window.webkitAudioContext)();
    console.debug("[audio] created AudioContext, state:", _audioCtx.state);
  }
  if (_audioCtx.state === "suspended") {
    console.debug("[audio] resuming suspended AudioContext");
    _audioCtx.resume().catch((e2) => console.warn("[audio] resume failed:", e2));
  }
}
let _activeAudio = null;
function isEditableTarget(el) {
  if (!(el && el instanceof HTMLElement)) return false;
  const tag = el.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  return el.isContentEditable;
}
document.addEventListener("keydown", (e2) => {
  if (e2.key !== " " || e2.repeat) return;
  if (isEditableTarget(e2.target)) return;
  if (!_activeAudio) return;
  e2.preventDefault();
  if (_activeAudio.paused) {
    _activeAudio.play().catch(() => void 0);
  } else {
    _activeAudio.pause();
  }
});
function renderAudioPlayer(container, audioSrc, autoplay) {
  const wrap = document.createElement("div");
  wrap.className = "waveform-player mt-2";
  const audio = document.createElement("audio");
  audio.preload = "auto";
  audio.src = audioSrc;
  const playBtn = document.createElement("button");
  playBtn.className = "waveform-play-btn";
  playBtn.type = "button";
  playBtn.appendChild(createPlaySvg());
  const barsWrap = document.createElement("div");
  barsWrap.className = "waveform-bars";
  const durEl = document.createElement("span");
  durEl.className = "waveform-duration";
  durEl.textContent = "00:00";
  wrap.appendChild(playBtn);
  wrap.appendChild(barsWrap);
  wrap.appendChild(durEl);
  container.appendChild(wrap);
  const bars = [];
  for (let i2 = 0; i2 < WAVEFORM_BAR_COUNT; i2++) {
    const bar = document.createElement("div");
    bar.className = "waveform-bar";
    bar.style.height = "20%";
    barsWrap.appendChild(bar);
    bars.push(bar);
  }
  extractWaveform(audioSrc, WAVEFORM_BAR_COUNT).then((peaks) => {
    peaks.forEach((p2, idx) => {
      bars[idx].style.height = `${p2 * 100}%`;
    });
  }).catch(() => {
    for (const b2 of bars) {
      b2.style.height = `${20 + Math.random() * 60}%`;
    }
  });
  function syncDurationLabel() {
    if (!Number.isFinite(audio.duration) || audio.duration < 0) return;
    durEl.textContent = formatAudioDuration(audio.duration);
  }
  audio.addEventListener("loadedmetadata", syncDurationLabel);
  audio.addEventListener("durationchange", syncDurationLabel);
  audio.addEventListener("canplay", syncDurationLabel);
  playBtn.onclick = () => {
    if (audio.paused) {
      audio.play().catch(() => void 0);
    } else {
      audio.pause();
    }
  };
  let rafId = 0;
  let prevPlayed = -1;
  function tick() {
    if (!Number.isFinite(audio.duration) || audio.duration <= 0) {
      rafId = requestAnimationFrame(tick);
      return;
    }
    const progress = audio.currentTime / audio.duration;
    const playedCount = Math.floor(progress * WAVEFORM_BAR_COUNT);
    if (playedCount !== prevPlayed) {
      const lo = Math.min(playedCount, prevPlayed < 0 ? 0 : prevPlayed);
      const hi = Math.max(playedCount, prevPlayed < 0 ? WAVEFORM_BAR_COUNT : prevPlayed);
      for (let idx = lo; idx < hi; idx++) {
        bars[idx].classList.toggle("played", idx < playedCount);
      }
      prevPlayed = playedCount;
    }
    durEl.textContent = formatAudioDuration(audio.currentTime);
    rafId = requestAnimationFrame(tick);
  }
  audio.addEventListener("play", () => {
    _activeAudio = audio;
    playBtn.replaceChildren(createPauseSvg());
    prevPlayed = -1;
    rafId = requestAnimationFrame(tick);
  });
  audio.addEventListener("pause", () => {
    playBtn.replaceChildren(createPlaySvg());
    cancelAnimationFrame(rafId);
  });
  audio.addEventListener("ended", () => {
    if (_activeAudio === audio) _activeAudio = null;
    playBtn.replaceChildren(createPlaySvg());
    cancelAnimationFrame(rafId);
    for (const b2 of bars) b2.classList.remove("played");
    prevPlayed = -1;
    if (Number.isFinite(audio.duration) && audio.duration >= 0) {
      durEl.textContent = formatAudioDuration(audio.duration);
    }
  });
  barsWrap.onclick = (e2) => {
    if (!Number.isFinite(audio.duration) || audio.duration <= 0) return;
    const rect = barsWrap.getBoundingClientRect();
    const fraction = (e2.clientX - rect.left) / rect.width;
    audio.currentTime = Math.max(0, Math.min(1, fraction)) * audio.duration;
    if (audio.paused) audio.play().catch(() => void 0);
  };
  if (autoplay) {
    warmAudioPlayback();
    console.debug(
      "[audio] autoplay requested, readyState:",
      audio.readyState,
      "audioCtx:",
      _audioCtx == null ? void 0 : _audioCtx.state,
      "src:",
      audioSrc.substring(0, 60)
    );
    const doPlay = () => {
      console.debug("[audio] attempting play(), readyState:", audio.readyState, "paused:", audio.paused);
      audio.play().then(() => console.debug("[audio] play() succeeded")).catch((e2) => console.warn("[audio] play() rejected:", e2.name, e2.message));
    };
    if (audio.readyState >= 3) {
      doPlay();
    } else {
      console.debug("[audio] waiting for canplay event");
      audio.addEventListener("canplay", doPlay, { once: true });
    }
  }
}
function resolveMapUrl(links) {
  if (!(links && typeof links === "object")) return "";
  if (typeof links.url === "string" && links.url.trim()) return links.url.trim();
  const providers = ["google_maps", "apple_maps", "openstreetmap"];
  for (const provider of providers) {
    const providerUrl = links[provider];
    if (typeof providerUrl === "string" && providerUrl.trim()) return providerUrl.trim();
  }
  return "";
}
function mapPointHeading(point, index) {
  var _a2, _b;
  const label = typeof (point == null ? void 0 : point.label) === "string" ? point.label.trim() : "";
  if (label) return label;
  const latOk = typeof (point == null ? void 0 : point.latitude) === "number" && Number.isFinite(point.latitude);
  const lonOk = typeof (point == null ? void 0 : point.longitude) === "number" && Number.isFinite(point.longitude);
  if (latOk && lonOk) return `${(_a2 = point.latitude) == null ? void 0 : _a2.toFixed(5)}, ${(_b = point.longitude) == null ? void 0 : _b.toFixed(5)}`;
  return `Location ${index + 1}`;
}
function splitMapLinkText(text) {
  const normalized = typeof text === "string" ? text.trim() : "";
  if (!normalized) return { primary: "", secondary: "" };
  const starIndex = normalized.indexOf("⭐");
  if (starIndex <= 0) return { primary: normalized, secondary: "" };
  const primary = normalized.slice(0, starIndex).trim();
  const secondary = normalized.slice(starIndex).trim();
  if (!(primary && secondary)) return { primary: normalized, secondary: "" };
  return { primary, secondary };
}
function renderMapLinks(container, links, label, heading) {
  const mapUrl = resolveMapUrl(links);
  if (!mapUrl) return false;
  const block = document.createElement("div");
  block.className = "mt-2";
  const text = heading || (typeof label === "string" && label.trim() ? label.trim() : "Open map");
  const textParts = splitMapLinkText(text);
  const link = document.createElement("a");
  link.href = mapUrl;
  link.target = "_blank";
  link.rel = "noopener noreferrer";
  link.className = "text-xs map-link-row";
  const primary = document.createElement("span");
  primary.className = "map-link-name";
  primary.textContent = textParts.primary || text;
  link.appendChild(primary);
  if (textParts.secondary) {
    const secondary = document.createElement("span");
    secondary.className = "map-link-meta";
    secondary.textContent = textParts.secondary;
    link.appendChild(secondary);
  }
  link.title = `Open "${text}" in maps`;
  block.appendChild(link);
  container.appendChild(block);
  return true;
}
function renderMapPointGroups(container, points, fallbackLabel) {
  if (!Array.isArray(points) || points.length === 0) return false;
  let rendered = false;
  const showHeadings = points.length > 1;
  for (let i2 = 0; i2 < points.length; i2++) {
    const point = points[i2];
    if (!(point && typeof point === "object")) continue;
    const label = typeof point.label === "string" && point.label.trim() ? point.label.trim() : fallbackLabel;
    const heading = showHeadings ? mapPointHeading(point, i2) : "";
    if (renderMapLinks(container, point.map_links, label, heading)) rendered = true;
  }
  return rendered;
}
function parseAgentsListPayload(payload) {
  var _a2, _b;
  if (Array.isArray(payload)) {
    const legacyDefault = (_a2 = payload.find((agent) => (agent == null ? void 0 : agent.is_default) === true && typeof (agent == null ? void 0 : agent.id) === "string")) == null ? void 0 : _a2.id;
    return { defaultId: legacyDefault || "main", agents: payload };
  }
  const agents = Array.isArray(payload == null ? void 0 : payload.agents) ? payload.agents : [];
  const inferredDefault = (_b = agents.find((agent) => (agent == null ? void 0 : agent.is_default) === true && typeof (agent == null ? void 0 : agent.id) === "string")) == null ? void 0 : _b.id;
  return {
    defaultId: typeof (payload == null ? void 0 : payload.default_id) === "string" ? payload.default_id : inferredDefault || "main",
    agents
  };
}
function createEl(tag, attrs, children) {
  const el = document.createElement(tag);
  if (attrs) {
    Object.keys(attrs).forEach((k2) => {
      const value = attrs[k2];
      if (value === void 0) return;
      if (k2 === "className") el.className = value;
      else if (k2 === "textContent") el.textContent = value;
      else if (k2 === "style") el.style.cssText = value;
      else el.setAttribute(k2, value);
    });
  }
  if (children) {
    children.forEach((c2) => {
      if (c2) el.appendChild(c2);
    });
  }
  return el;
}
const _helpers = /* @__PURE__ */ Object.freeze(/* @__PURE__ */ Object.defineProperty({
  __proto__: null,
  createEl,
  esc,
  formatAssistantTokenUsage,
  formatAudioDuration,
  formatBytes,
  formatTokenSpeed,
  formatTokens,
  localizeRpcError,
  localizeStructuredError,
  localizedApiErrorMessage,
  localizedRpcErrorMessage,
  modelVersionScore,
  nextId,
  parseAgentsListPayload,
  parseErrorMessage,
  renderAudioPlayer,
  renderDocument,
  renderMapLinks,
  renderMapPointGroups,
  renderMarkdown,
  renderScreenshot,
  sendRpc,
  tokenSpeedPerSecond,
  tokenSpeedTone,
  toolCallSummary,
  updateCountdown,
  warmAudioPlayback
}, Symbol.toStringTag, { value: "Module" }));
function getSystemTheme() {
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}
function applyTheme(mode) {
  const resolved = mode === "system" ? getSystemTheme() : mode;
  document.documentElement.setAttribute("data-theme", resolved);
  document.documentElement.style.colorScheme = resolved;
  updateThemeButtons(mode);
}
function updateThemeButtons(activeMode) {
  const buttons = document.querySelectorAll(".theme-btn");
  buttons.forEach((btn) => {
    btn.classList.toggle("active", btn.getAttribute("data-theme-val") === activeMode);
  });
}
function initTheme() {
  const saved = localStorage.getItem("moltis-theme") || "system";
  applyTheme(saved);
  const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
  const onSystemThemeChange = () => {
    const current = localStorage.getItem("moltis-theme") || "system";
    if (current === "system") applyTheme("system");
  };
  if (typeof mediaQuery.addEventListener === "function") {
    mediaQuery.addEventListener("change", onSystemThemeChange);
  } else if (typeof mediaQuery.addListener === "function") {
    mediaQuery.addListener(onSystemThemeChange);
  }
  const themeToggle = $("themeToggle");
  if (!themeToggle) return;
  themeToggle.addEventListener("click", (e2) => {
    const btn = e2.target.closest(".theme-btn");
    if (!btn) return;
    const mode = btn.getAttribute("data-theme-val");
    if (!mode) return;
    localStorage.setItem("moltis-theme", mode);
    applyTheme(mode);
  });
}
function injectMarkdownStyles() {
  const ms = document.createElement("style");
  ms.textContent = ".skill-body-md h1{font-size:1.25rem;font-weight:700;margin:16px 0 8px;padding-bottom:4px;border-bottom:1px solid var(--border)}.skill-body-md h2{font-size:1.1rem;font-weight:600;margin:14px 0 6px;padding-bottom:3px;border-bottom:1px solid var(--border)}.skill-body-md h3{font-size:.95rem;font-weight:600;margin:12px 0 4px}.skill-body-md h4{font-size:.88rem;font-weight:600;margin:10px 0 4px}.skill-body-md h5,.skill-body-md h6{font-size:.82rem;font-weight:600;margin:8px 0 4px}.skill-body-md p{margin:6px 0;line-height:1.6}.skill-body-md ul,.skill-body-md ol{margin:6px 0 6px 20px;padding:0}.skill-body-md ul{list-style:disc}.skill-body-md ol{list-style:decimal}.skill-body-md li{margin:2px 0;line-height:1.5}.skill-body-md li>ul,.skill-body-md li>ol{margin:2px 0 2px 16px}.skill-body-md code{background:var(--surface);padding:1px 5px;border-radius:4px;font-size:.82em;font-family:var(--font-mono)}.skill-body-md pre{background:var(--surface);border:1px solid var(--border);border-radius:var(--radius-sm);padding:10px 12px;overflow-x:auto;margin:8px 0;line-height:1.45}.skill-body-md pre code{background:none;padding:0;font-size:.78rem}.skill-body-md blockquote{border-left:3px solid var(--border);margin:8px 0;padding:4px 12px;color:var(--muted)}.skill-body-md a{color:var(--accent);text-decoration:underline}.skill-body-md a:hover{opacity:.8}.skill-body-md hr{border:none;border-top:1px solid var(--border);margin:12px 0}.skill-body-md table{border-collapse:collapse;width:100%;margin:8px 0;font-size:.8rem}.skill-body-md th,.skill-body-md td{border:1px solid var(--border);padding:5px 8px;text-align:left}.skill-body-md th{background:var(--surface);font-weight:600}.skill-body-md strong{font-weight:600}.skill-body-md em{font-style:italic}.skill-body-md img{max-width:100%;border-radius:var(--radius-sm)}.skill-body-md input[type=checkbox]{margin-right:4px}";
  document.head.appendChild(ms);
}
export {
  $,
  renderMarkdown as A,
  setSessionContextWindow as B,
  setSessionTokens as C,
  setSessionCurrentInputTokens as D,
  setSessionToolsEnabled as E,
  toolCallSummary as F,
  renderScreenshot as G,
  renderDocument as H,
  formatAssistantTokenUsage as I,
  formatTokenSpeed as J,
  tokenSpeedTone as K,
  modelStore as L,
  parseAgentsListPayload as M,
  setHostExecIsRoot as N,
  setSessionExecMode as O,
  setSessionExecPromptSymbol as P,
  setChatBatchLoading as Q,
  setChatSeq as R,
  y$1 as S,
  g$1 as T,
  nodeComboBtn as U,
  nodeDropdownList as V,
  nodeCombo as W,
  nodeDropdown as X,
  nodeComboLabel as Y,
  projectComboLabel as Z,
  __vitePreload as _,
  chatInput as a,
  setNodeComboBtn as a$,
  t as a0,
  projects as a1,
  activeProjectId as a2,
  projectCombo as a3,
  projectDropdown as a4,
  projectDropdownList as a5,
  setActiveProjectId as a6,
  projectComboBtn as a7,
  j as a8,
  setSessionSandboxEnabled as a9,
  S$2 as aA,
  projectFilterId$1 as aB,
  getById as aC,
  q$1 as aD,
  setProjects as aE,
  setProjectFilterId as aF,
  warmAudioPlayback as aG,
  selectedModelId as aH,
  formatBytes as aI,
  setCommandModeEnabled as aJ,
  chatHistory as aK,
  chatHistoryIdx as aL,
  setChatHistoryDraft as aM,
  setChatHistoryIdx as aN,
  chatHistoryDraft as aO,
  setChatHistory as aP,
  R as aQ,
  setChatMsgBox as aR,
  setChatInput as aS,
  setChatSendBtn as aT,
  setModelCombo as aU,
  setModelComboBtn as aV,
  setModelComboLabel as aW,
  setModelDropdown as aX,
  setModelSearchInput as aY,
  setModelDropdownList as aZ,
  setNodeCombo as a_,
  hostExecIsRoot as aa,
  sandboxLabel as ab,
  sandboxToggleBtn as ac,
  sessionSandboxEnabled as ad,
  setSessionSandboxImage as ae,
  sandboxImageLabel as af,
  sandboxInfo as ag,
  sandboxImageDropdown as ah,
  sandboxImageBtn as ai,
  sessionSandboxImage as aj,
  projectStore as ak,
  setSessions as al,
  insertSessionInOrder as am,
  Session as an,
  chatSeq as ao,
  setSelectedModelId as ap,
  modelComboLabel as aq,
  setSessionSwitchInProgress as ar,
  setStreamEl as as,
  setStreamText as at,
  setLastToolOutput as au,
  setVoicePending as av,
  setActiveSessionKey as aw,
  y$2 as ax,
  d$2 as ay,
  A as az,
  sendRpc as b,
  setAll$2 as b$,
  setNodeComboLabel as b0,
  setNodeDropdown as b1,
  setNodeDropdownList as b2,
  setSandboxToggleBtn as b3,
  setSandboxLabel as b4,
  setProjectCombo as b5,
  setProjectComboBtn as b6,
  setProjectComboLabel as b7,
  setProjectDropdown as b8,
  setProjectDropdownList as b9,
  renderMapLinks as bA,
  lastToolOutput as bB,
  localizeStructuredError as bC,
  voicePending as bD,
  streamText as bE,
  setSandboxInfo as bF,
  networkAuditEventHandler as bG,
  logsEventHandler as bH,
  setSubscribed as bI,
  projects$1 as bJ,
  sandboxInfo$1 as bK,
  localizedApiErrorMessage as bL,
  setLogsEventHandler as bM,
  setNetworkAuditEventHandler as bN,
  setRefreshProvidersPage as bO,
  setLocale as bP,
  esc as bQ,
  projectStore$1 as bR,
  _modelStore as bS,
  S as bT,
  _sessionStoreModule as bU,
  _i18n as bV,
  _helpers as bW,
  initTheme as bX,
  injectMarkdownStyles as bY,
  init as bZ,
  translateStaticElements as b_,
  setSandboxImageBtn as ba,
  setSandboxImageLabel as bb,
  setSandboxImageDropdown as bc,
  models as bd,
  chatSendBtn as be,
  setModels as bf,
  modelComboBtn as bg,
  modelSearchInput as bh,
  modelDropdownList as bi,
  modelCombo as bj,
  modelDropdown as bk,
  setModelIdx as bl,
  modelIdx as bm,
  REASONING_SEP as bn,
  models$1 as bo,
  useSignal as bp,
  connected$1 as bq,
  setCachedChannels as br,
  setRefreshChannelsPage as bs,
  cachedChannels as bt,
  setChannelEventUnsub as bu,
  channelEventUnsub as bv,
  refreshProvidersPage as bw,
  modelVersionScore as bx,
  streamEl as by,
  renderMapPointGroups as bz,
  chatMsgBox as c,
  setAll$1 as c0,
  select as c1,
  selectedModelId$1 as c2,
  l$3 as c3,
  localizeRpcError as c4,
  pending as c5,
  setConnected as c6,
  nextId as c7,
  getPreferredLocale as c8,
  setReconnectDelay as c9,
  reconnectDelay as ca,
  setWs as cb,
  autoScrollMode as d,
  commandModeEnabled as e,
  formatTokens as f,
  sessionExecPromptSymbol as g,
  chatBatchLoading as h,
  sessionContextWindow as i,
  sessionToolsEnabled as j,
  sessionExecMode as k,
  sessionCurrentInputTokens as l,
  setUnseenErrors as m,
  setUnseenWarns as n,
  unseenErrors as o,
  parseErrorMessage as p,
  unseenWarns as q,
  connected as r,
  sessionTokens as s,
  sessionStore as t,
  updateCountdown as u,
  sessions as v,
  activeSessionKey as w,
  lastHistoryIndex as x,
  setLastHistoryIndex as y,
  renderAudioPlayer as z
};
