import { bV as l } from "./theme.js";
var f = 0;
function u(e, t, n, o, i, u2) {
  t || (t = {});
  var a, c, p = t;
  if ("ref" in p) for (c in p = {}, t) "ref" == c ? a = t[c] : p[c] = t[c];
  var l$1 = { type: e, props: p, key: n, ref: a, __k: null, __: null, __b: 0, __e: null, __c: null, constructor: void 0, __v: --f, __i: -1, __u: 0, __source: i, __self: u2 };
  if ("function" == typeof e && (a = e.defaultProps)) for (c in a) void 0 === p[c] && (p[c] = a[c]);
  return l.vnode && l.vnode(l$1), l$1;
}
export {
  u
};
