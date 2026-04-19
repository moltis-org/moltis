const __vite__mapDeps=(i,m=__vite__mapDeps,d=(m.f||(m.f=["chunks/angular-html.js","chunks/html.js","chunks/javascript.js","chunks/css.js","chunks/angular-ts.js","chunks/scss.js","chunks/apl.js","chunks/xml.js","chunks/java.js","chunks/json.js","chunks/astro.js","chunks/typescript.js","chunks/postcss.js","chunks/tsx.js","chunks/blade.js","chunks/html-derivative.js","chunks/sql.js","chunks/bsl.js","chunks/sdbl.js","chunks/cairo.js","chunks/python.js","chunks/cobol.js","chunks/coffee.js","chunks/cpp.js","chunks/regexp.js","chunks/glsl.js","chunks/c.js","chunks/crystal.js","chunks/shellscript.js","chunks/edge.js","chunks/elixir.js","chunks/elm.js","chunks/erb.js","chunks/ruby.js","chunks/haml.js","chunks/graphql.js","chunks/jsx.js","chunks/lua.js","chunks/yaml.js","chunks/erlang.js","chunks/markdown.js","chunks/fortran-fixed-form.js","chunks/fortran-free-form.js","chunks/fsharp.js","chunks/gdresource.js","chunks/gdshader.js","chunks/gdscript.js","chunks/git-commit.js","chunks/diff.js","chunks/git-rebase.js","chunks/glimmer-js.js","chunks/glimmer-ts.js","chunks/hack.js","chunks/handlebars.js","chunks/http.js","chunks/hurl.js","chunks/csv.js","chunks/hxml.js","chunks/haxe.js","chunks/jinja.js","chunks/jison.js","chunks/julia.js","chunks/r.js","chunks/just.js","chunks/perl.js","chunks/latex.js","chunks/tex.js","chunks/liquid.js","chunks/marko.js","chunks/less.js","chunks/mdc.js","chunks/nextflow.js","chunks/nextflow-groovy.js","chunks/nginx.js","chunks/nim.js","chunks/php.js","chunks/pug.js","chunks/qml.js","chunks/razor.js","chunks/csharp.js","chunks/rst.js","chunks/cmake.js","chunks/sas.js","chunks/shaderlab.js","chunks/hlsl.js","chunks/shellsession.js","chunks/soy.js","chunks/sparql.js","chunks/turtle.js","chunks/stata.js","chunks/surrealql.js","chunks/svelte.js","chunks/templ.js","chunks/go.js","chunks/ts-tags.js","chunks/twig.js","chunks/vue.js","chunks/vue-html.js","chunks/vue-vine.js","chunks/stylus.js","chunks/xsl.js"])))=>i.map(i=>d[i]);
var __defProp = Object.defineProperty;
var __getProtoOf = Object.getPrototypeOf;
var __reflectGet = Reflect.get;
var __typeError = (msg) => {
  throw TypeError(msg);
};
var __defNormalProp = (obj, key2, value) => key2 in obj ? __defProp(obj, key2, { enumerable: true, configurable: true, writable: true, value }) : obj[key2] = value;
var __publicField = (obj, key2, value) => __defNormalProp(obj, typeof key2 !== "symbol" ? key2 + "" : key2, value);
var __accessCheck = (obj, member, msg) => member.has(obj) || __typeError("Cannot " + msg);
var __privateGet = (obj, member, getter) => (__accessCheck(obj, member, "read from private field"), getter ? getter.call(obj) : member.get(obj));
var __privateAdd = (obj, member, value) => member.has(obj) ? __typeError("Cannot add the same private member more than once") : member instanceof WeakSet ? member.add(obj) : member.set(obj, value);
var __privateSet = (obj, member, value, setter) => (__accessCheck(obj, member, "write to private field"), setter ? setter.call(obj, value) : member.set(obj, value), value);
var __privateMethod = (obj, member, method) => (__accessCheck(obj, member, "access private method"), method);
var __superGet = (cls, obj, key2) => __reflectGet(__getProtoOf(cls), key2, obj);
var _a, _b, _captureMap, _compiled, _pattern, _nameMap, _strategy, __EmulatedRegExp_instances, execCore_fn, _c;
import { _ as __vitePreload } from "./theme.js";
let ShikiError$2 = class ShikiError extends Error {
  constructor(message) {
    super(message);
    this.name = "ShikiError";
  }
};
function clone(something) {
  return doClone(something);
}
function doClone(something) {
  if (Array.isArray(something)) {
    return cloneArray(something);
  }
  if (something instanceof RegExp) {
    return something;
  }
  if (typeof something === "object") {
    return cloneObj(something);
  }
  return something;
}
function cloneArray(arr) {
  let r2 = [];
  for (let i2 = 0, len = arr.length; i2 < len; i2++) {
    r2[i2] = doClone(arr[i2]);
  }
  return r2;
}
function cloneObj(obj) {
  let r2 = {};
  for (let key2 in obj) {
    r2[key2] = doClone(obj[key2]);
  }
  return r2;
}
function mergeObjects(target, ...sources) {
  sources.forEach((source) => {
    for (let key2 in source) {
      target[key2] = source[key2];
    }
  });
  return target;
}
function basename(path) {
  const idx = ~path.lastIndexOf("/") || ~path.lastIndexOf("\\");
  if (idx === 0) {
    return path;
  } else if (~idx === path.length - 1) {
    return basename(path.substring(0, path.length - 1));
  } else {
    return path.substr(~idx + 1);
  }
}
var CAPTURING_REGEX_SOURCE = /\$(\d+)|\${(\d+):\/(downcase|upcase)}/g;
var RegexSource = class {
  static hasCaptures(regexSource) {
    if (regexSource === null) {
      return false;
    }
    CAPTURING_REGEX_SOURCE.lastIndex = 0;
    return CAPTURING_REGEX_SOURCE.test(regexSource);
  }
  static replaceCaptures(regexSource, captureSource, captureIndices) {
    return regexSource.replace(CAPTURING_REGEX_SOURCE, (match, index, commandIndex, command) => {
      let capture = captureIndices[parseInt(index || commandIndex, 10)];
      if (capture) {
        let result = captureSource.substring(capture.start, capture.end);
        while (result[0] === ".") {
          result = result.substring(1);
        }
        switch (command) {
          case "downcase":
            return result.toLowerCase();
          case "upcase":
            return result.toUpperCase();
          default:
            return result;
        }
      } else {
        return match;
      }
    });
  }
};
function strcmp(a, b2) {
  if (a < b2) {
    return -1;
  }
  if (a > b2) {
    return 1;
  }
  return 0;
}
function strArrCmp(a, b2) {
  if (a === null && b2 === null) {
    return 0;
  }
  if (!a) {
    return -1;
  }
  if (!b2) {
    return 1;
  }
  let len1 = a.length;
  let len2 = b2.length;
  if (len1 === len2) {
    for (let i2 = 0; i2 < len1; i2++) {
      let res = strcmp(a[i2], b2[i2]);
      if (res !== 0) {
        return res;
      }
    }
    return 0;
  }
  return len1 - len2;
}
function isValidHexColor(hex) {
  if (/^#[0-9a-f]{6}$/i.test(hex)) {
    return true;
  }
  if (/^#[0-9a-f]{8}$/i.test(hex)) {
    return true;
  }
  if (/^#[0-9a-f]{3}$/i.test(hex)) {
    return true;
  }
  if (/^#[0-9a-f]{4}$/i.test(hex)) {
    return true;
  }
  return false;
}
function escapeRegExpCharacters(value) {
  return value.replace(/[\-\\\{\}\*\+\?\|\^\$\.\,\[\]\(\)\#\s]/g, "\\$&");
}
var CachedFn = class {
  constructor(fn) {
    __publicField(this, "cache", /* @__PURE__ */ new Map());
    this.fn = fn;
  }
  get(key2) {
    if (this.cache.has(key2)) {
      return this.cache.get(key2);
    }
    const value = this.fn(key2);
    this.cache.set(key2, value);
    return value;
  }
};
var Theme = class {
  constructor(_colorMap, _defaults, _root) {
    __publicField(this, "_cachedMatchRoot", new CachedFn(
      (scopeName) => this._root.match(scopeName)
    ));
    this._colorMap = _colorMap;
    this._defaults = _defaults;
    this._root = _root;
  }
  static createFromRawTheme(source, colorMap) {
    return this.createFromParsedTheme(parseTheme(source), colorMap);
  }
  static createFromParsedTheme(source, colorMap) {
    return resolveParsedThemeRules(source, colorMap);
  }
  getColorMap() {
    return this._colorMap.getColorMap();
  }
  getDefaults() {
    return this._defaults;
  }
  match(scopePath) {
    if (scopePath === null) {
      return this._defaults;
    }
    const scopeName = scopePath.scopeName;
    const matchingTrieElements = this._cachedMatchRoot.get(scopeName);
    const effectiveRule = matchingTrieElements.find(
      (v2) => _scopePathMatchesParentScopes(scopePath.parent, v2.parentScopes)
    );
    if (!effectiveRule) {
      return null;
    }
    return new StyleAttributes(
      effectiveRule.fontStyle,
      effectiveRule.foreground,
      effectiveRule.background
    );
  }
};
var ScopeStack = class _ScopeStack {
  constructor(parent, scopeName) {
    this.parent = parent;
    this.scopeName = scopeName;
  }
  static push(path, scopeNames) {
    for (const name of scopeNames) {
      path = new _ScopeStack(path, name);
    }
    return path;
  }
  static from(...segments) {
    let result = null;
    for (let i2 = 0; i2 < segments.length; i2++) {
      result = new _ScopeStack(result, segments[i2]);
    }
    return result;
  }
  push(scopeName) {
    return new _ScopeStack(this, scopeName);
  }
  getSegments() {
    let item = this;
    const result = [];
    while (item) {
      result.push(item.scopeName);
      item = item.parent;
    }
    result.reverse();
    return result;
  }
  toString() {
    return this.getSegments().join(" ");
  }
  extends(other) {
    if (this === other) {
      return true;
    }
    if (this.parent === null) {
      return false;
    }
    return this.parent.extends(other);
  }
  getExtensionIfDefined(base) {
    const result = [];
    let item = this;
    while (item && item !== base) {
      result.push(item.scopeName);
      item = item.parent;
    }
    return item === base ? result.reverse() : void 0;
  }
};
function _scopePathMatchesParentScopes(scopePath, parentScopes) {
  if (parentScopes.length === 0) {
    return true;
  }
  for (let index = 0; index < parentScopes.length; index++) {
    let scopePattern = parentScopes[index];
    let scopeMustMatch = false;
    if (scopePattern === ">") {
      if (index === parentScopes.length - 1) {
        return false;
      }
      scopePattern = parentScopes[++index];
      scopeMustMatch = true;
    }
    while (scopePath) {
      if (_matchesScope(scopePath.scopeName, scopePattern)) {
        break;
      }
      if (scopeMustMatch) {
        return false;
      }
      scopePath = scopePath.parent;
    }
    if (!scopePath) {
      return false;
    }
    scopePath = scopePath.parent;
  }
  return true;
}
function _matchesScope(scopeName, scopePattern) {
  return scopePattern === scopeName || scopeName.startsWith(scopePattern) && scopeName[scopePattern.length] === ".";
}
var StyleAttributes = class {
  constructor(fontStyle, foregroundId, backgroundId) {
    this.fontStyle = fontStyle;
    this.foregroundId = foregroundId;
    this.backgroundId = backgroundId;
  }
};
function parseTheme(source) {
  if (!source) {
    return [];
  }
  if (!source.settings || !Array.isArray(source.settings)) {
    return [];
  }
  let settings = source.settings;
  let result = [], resultLen = 0;
  for (let i2 = 0, len = settings.length; i2 < len; i2++) {
    let entry = settings[i2];
    if (!entry.settings) {
      continue;
    }
    let scopes;
    if (typeof entry.scope === "string") {
      let _scope = entry.scope;
      _scope = _scope.replace(/^[,]+/, "");
      _scope = _scope.replace(/[,]+$/, "");
      scopes = _scope.split(",");
    } else if (Array.isArray(entry.scope)) {
      scopes = entry.scope;
    } else {
      scopes = [""];
    }
    let fontStyle = -1;
    if (typeof entry.settings.fontStyle === "string") {
      fontStyle = 0;
      let segments = entry.settings.fontStyle.split(" ");
      for (let j2 = 0, lenJ = segments.length; j2 < lenJ; j2++) {
        let segment = segments[j2];
        switch (segment) {
          case "italic":
            fontStyle = fontStyle | 1;
            break;
          case "bold":
            fontStyle = fontStyle | 2;
            break;
          case "underline":
            fontStyle = fontStyle | 4;
            break;
          case "strikethrough":
            fontStyle = fontStyle | 8;
            break;
        }
      }
    }
    let foreground = null;
    if (typeof entry.settings.foreground === "string" && isValidHexColor(entry.settings.foreground)) {
      foreground = entry.settings.foreground;
    }
    let background = null;
    if (typeof entry.settings.background === "string" && isValidHexColor(entry.settings.background)) {
      background = entry.settings.background;
    }
    for (let j2 = 0, lenJ = scopes.length; j2 < lenJ; j2++) {
      let _scope = scopes[j2].trim();
      let segments = _scope.split(" ");
      let scope = segments[segments.length - 1];
      let parentScopes = null;
      if (segments.length > 1) {
        parentScopes = segments.slice(0, segments.length - 1);
        parentScopes.reverse();
      }
      result[resultLen++] = new ParsedThemeRule(
        scope,
        parentScopes,
        i2,
        fontStyle,
        foreground,
        background
      );
    }
  }
  return result;
}
var ParsedThemeRule = class {
  constructor(scope, parentScopes, index, fontStyle, foreground, background) {
    this.scope = scope;
    this.parentScopes = parentScopes;
    this.index = index;
    this.fontStyle = fontStyle;
    this.foreground = foreground;
    this.background = background;
  }
};
var FontStyle = /* @__PURE__ */ ((FontStyle2) => {
  FontStyle2[FontStyle2["NotSet"] = -1] = "NotSet";
  FontStyle2[FontStyle2["None"] = 0] = "None";
  FontStyle2[FontStyle2["Italic"] = 1] = "Italic";
  FontStyle2[FontStyle2["Bold"] = 2] = "Bold";
  FontStyle2[FontStyle2["Underline"] = 4] = "Underline";
  FontStyle2[FontStyle2["Strikethrough"] = 8] = "Strikethrough";
  return FontStyle2;
})(FontStyle || {});
function resolveParsedThemeRules(parsedThemeRules, _colorMap) {
  parsedThemeRules.sort((a, b2) => {
    let r2 = strcmp(a.scope, b2.scope);
    if (r2 !== 0) {
      return r2;
    }
    r2 = strArrCmp(a.parentScopes, b2.parentScopes);
    if (r2 !== 0) {
      return r2;
    }
    return a.index - b2.index;
  });
  let defaultFontStyle = 0;
  let defaultForeground = "#000000";
  let defaultBackground = "#ffffff";
  while (parsedThemeRules.length >= 1 && parsedThemeRules[0].scope === "") {
    let incomingDefaults = parsedThemeRules.shift();
    if (incomingDefaults.fontStyle !== -1) {
      defaultFontStyle = incomingDefaults.fontStyle;
    }
    if (incomingDefaults.foreground !== null) {
      defaultForeground = incomingDefaults.foreground;
    }
    if (incomingDefaults.background !== null) {
      defaultBackground = incomingDefaults.background;
    }
  }
  let colorMap = new ColorMap(_colorMap);
  let defaults = new StyleAttributes(defaultFontStyle, colorMap.getId(defaultForeground), colorMap.getId(defaultBackground));
  let root2 = new ThemeTrieElement(new ThemeTrieElementRule(0, null, -1, 0, 0), []);
  for (let i2 = 0, len = parsedThemeRules.length; i2 < len; i2++) {
    let rule = parsedThemeRules[i2];
    root2.insert(0, rule.scope, rule.parentScopes, rule.fontStyle, colorMap.getId(rule.foreground), colorMap.getId(rule.background));
  }
  return new Theme(colorMap, defaults, root2);
}
var ColorMap = class {
  constructor(_colorMap) {
    __publicField(this, "_isFrozen");
    __publicField(this, "_lastColorId");
    __publicField(this, "_id2color");
    __publicField(this, "_color2id");
    this._lastColorId = 0;
    this._id2color = [];
    this._color2id = /* @__PURE__ */ Object.create(null);
    if (Array.isArray(_colorMap)) {
      this._isFrozen = true;
      for (let i2 = 0, len = _colorMap.length; i2 < len; i2++) {
        this._color2id[_colorMap[i2]] = i2;
        this._id2color[i2] = _colorMap[i2];
      }
    } else {
      this._isFrozen = false;
    }
  }
  getId(color) {
    if (color === null) {
      return 0;
    }
    color = color.toUpperCase();
    let value = this._color2id[color];
    if (value) {
      return value;
    }
    if (this._isFrozen) {
      throw new Error(`Missing color in color map - ${color}`);
    }
    value = ++this._lastColorId;
    this._color2id[color] = value;
    this._id2color[value] = color;
    return value;
  }
  getColorMap() {
    return this._id2color.slice(0);
  }
};
var emptyParentScopes = Object.freeze([]);
var ThemeTrieElementRule = class _ThemeTrieElementRule {
  constructor(scopeDepth, parentScopes, fontStyle, foreground, background) {
    __publicField(this, "scopeDepth");
    __publicField(this, "parentScopes");
    __publicField(this, "fontStyle");
    __publicField(this, "foreground");
    __publicField(this, "background");
    this.scopeDepth = scopeDepth;
    this.parentScopes = parentScopes || emptyParentScopes;
    this.fontStyle = fontStyle;
    this.foreground = foreground;
    this.background = background;
  }
  clone() {
    return new _ThemeTrieElementRule(this.scopeDepth, this.parentScopes, this.fontStyle, this.foreground, this.background);
  }
  static cloneArr(arr) {
    let r2 = [];
    for (let i2 = 0, len = arr.length; i2 < len; i2++) {
      r2[i2] = arr[i2].clone();
    }
    return r2;
  }
  acceptOverwrite(scopeDepth, fontStyle, foreground, background) {
    if (this.scopeDepth > scopeDepth) {
      console.log("how did this happen?");
    } else {
      this.scopeDepth = scopeDepth;
    }
    if (fontStyle !== -1) {
      this.fontStyle = fontStyle;
    }
    if (foreground !== 0) {
      this.foreground = foreground;
    }
    if (background !== 0) {
      this.background = background;
    }
  }
};
var ThemeTrieElement = class _ThemeTrieElement {
  constructor(_mainRule, rulesWithParentScopes = [], _children = {}) {
    __publicField(this, "_rulesWithParentScopes");
    this._mainRule = _mainRule;
    this._children = _children;
    this._rulesWithParentScopes = rulesWithParentScopes;
  }
  static _cmpBySpecificity(a, b2) {
    if (a.scopeDepth !== b2.scopeDepth) {
      return b2.scopeDepth - a.scopeDepth;
    }
    let aParentIndex = 0;
    let bParentIndex = 0;
    while (true) {
      if (a.parentScopes[aParentIndex] === ">") {
        aParentIndex++;
      }
      if (b2.parentScopes[bParentIndex] === ">") {
        bParentIndex++;
      }
      if (aParentIndex >= a.parentScopes.length || bParentIndex >= b2.parentScopes.length) {
        break;
      }
      const parentScopeLengthDiff = b2.parentScopes[bParentIndex].length - a.parentScopes[aParentIndex].length;
      if (parentScopeLengthDiff !== 0) {
        return parentScopeLengthDiff;
      }
      aParentIndex++;
      bParentIndex++;
    }
    return b2.parentScopes.length - a.parentScopes.length;
  }
  match(scope) {
    if (scope !== "") {
      let dotIndex = scope.indexOf(".");
      let head2;
      let tail;
      if (dotIndex === -1) {
        head2 = scope;
        tail = "";
      } else {
        head2 = scope.substring(0, dotIndex);
        tail = scope.substring(dotIndex + 1);
      }
      if (this._children.hasOwnProperty(head2)) {
        return this._children[head2].match(tail);
      }
    }
    const rules = this._rulesWithParentScopes.concat(this._mainRule);
    rules.sort(_ThemeTrieElement._cmpBySpecificity);
    return rules;
  }
  insert(scopeDepth, scope, parentScopes, fontStyle, foreground, background) {
    if (scope === "") {
      this._doInsertHere(scopeDepth, parentScopes, fontStyle, foreground, background);
      return;
    }
    let dotIndex = scope.indexOf(".");
    let head2;
    let tail;
    if (dotIndex === -1) {
      head2 = scope;
      tail = "";
    } else {
      head2 = scope.substring(0, dotIndex);
      tail = scope.substring(dotIndex + 1);
    }
    let child;
    if (this._children.hasOwnProperty(head2)) {
      child = this._children[head2];
    } else {
      child = new _ThemeTrieElement(this._mainRule.clone(), ThemeTrieElementRule.cloneArr(this._rulesWithParentScopes));
      this._children[head2] = child;
    }
    child.insert(scopeDepth + 1, tail, parentScopes, fontStyle, foreground, background);
  }
  _doInsertHere(scopeDepth, parentScopes, fontStyle, foreground, background) {
    if (parentScopes === null) {
      this._mainRule.acceptOverwrite(scopeDepth, fontStyle, foreground, background);
      return;
    }
    for (let i2 = 0, len = this._rulesWithParentScopes.length; i2 < len; i2++) {
      let rule = this._rulesWithParentScopes[i2];
      if (strArrCmp(rule.parentScopes, parentScopes) === 0) {
        rule.acceptOverwrite(scopeDepth, fontStyle, foreground, background);
        return;
      }
    }
    if (fontStyle === -1) {
      fontStyle = this._mainRule.fontStyle;
    }
    if (foreground === 0) {
      foreground = this._mainRule.foreground;
    }
    if (background === 0) {
      background = this._mainRule.background;
    }
    this._rulesWithParentScopes.push(new ThemeTrieElementRule(scopeDepth, parentScopes, fontStyle, foreground, background));
  }
};
var EncodedTokenMetadata = class _EncodedTokenMetadata {
  static toBinaryStr(encodedTokenAttributes) {
    return encodedTokenAttributes.toString(2).padStart(32, "0");
  }
  static print(encodedTokenAttributes) {
    const languageId = _EncodedTokenMetadata.getLanguageId(encodedTokenAttributes);
    const tokenType = _EncodedTokenMetadata.getTokenType(encodedTokenAttributes);
    const fontStyle = _EncodedTokenMetadata.getFontStyle(encodedTokenAttributes);
    const foreground = _EncodedTokenMetadata.getForeground(encodedTokenAttributes);
    const background = _EncodedTokenMetadata.getBackground(encodedTokenAttributes);
    console.log({
      languageId,
      tokenType,
      fontStyle,
      foreground,
      background
    });
  }
  static getLanguageId(encodedTokenAttributes) {
    return (encodedTokenAttributes & 255) >>> 0;
  }
  static getTokenType(encodedTokenAttributes) {
    return (encodedTokenAttributes & 768) >>> 8;
  }
  static containsBalancedBrackets(encodedTokenAttributes) {
    return (encodedTokenAttributes & 1024) !== 0;
  }
  static getFontStyle(encodedTokenAttributes) {
    return (encodedTokenAttributes & 30720) >>> 11;
  }
  static getForeground(encodedTokenAttributes) {
    return (encodedTokenAttributes & 16744448) >>> 15;
  }
  static getBackground(encodedTokenAttributes) {
    return (encodedTokenAttributes & 4278190080) >>> 24;
  }
  /**
   * Updates the fields in `metadata`.
   * A value of `0`, `NotSet` or `null` indicates that the corresponding field should be left as is.
   */
  static set(encodedTokenAttributes, languageId, tokenType, containsBalancedBrackets, fontStyle, foreground, background) {
    let _languageId = _EncodedTokenMetadata.getLanguageId(encodedTokenAttributes);
    let _tokenType = _EncodedTokenMetadata.getTokenType(encodedTokenAttributes);
    let _containsBalancedBracketsBit = _EncodedTokenMetadata.containsBalancedBrackets(encodedTokenAttributes) ? 1 : 0;
    let _fontStyle = _EncodedTokenMetadata.getFontStyle(encodedTokenAttributes);
    let _foreground = _EncodedTokenMetadata.getForeground(encodedTokenAttributes);
    let _background = _EncodedTokenMetadata.getBackground(encodedTokenAttributes);
    if (languageId !== 0) {
      _languageId = languageId;
    }
    if (tokenType !== 8) {
      _tokenType = fromOptionalTokenType(tokenType);
    }
    if (containsBalancedBrackets !== null) {
      _containsBalancedBracketsBit = containsBalancedBrackets ? 1 : 0;
    }
    if (fontStyle !== -1) {
      _fontStyle = fontStyle;
    }
    if (foreground !== 0) {
      _foreground = foreground;
    }
    if (background !== 0) {
      _background = background;
    }
    return (_languageId << 0 | _tokenType << 8 | _containsBalancedBracketsBit << 10 | _fontStyle << 11 | _foreground << 15 | _background << 24) >>> 0;
  }
};
function toOptionalTokenType(standardType) {
  return standardType;
}
function fromOptionalTokenType(standardType) {
  return standardType;
}
function createMatchers(selector, matchesName) {
  const results = [];
  const tokenizer = newTokenizer(selector);
  let token2 = tokenizer.next();
  while (token2 !== null) {
    let priority = 0;
    if (token2.length === 2 && token2.charAt(1) === ":") {
      switch (token2.charAt(0)) {
        case "R":
          priority = 1;
          break;
        case "L":
          priority = -1;
          break;
        default:
          console.log(`Unknown priority ${token2} in scope selector`);
      }
      token2 = tokenizer.next();
    }
    let matcher = parseConjunction();
    results.push({ matcher, priority });
    if (token2 !== ",") {
      break;
    }
    token2 = tokenizer.next();
  }
  return results;
  function parseOperand() {
    if (token2 === "-") {
      token2 = tokenizer.next();
      const expressionToNegate = parseOperand();
      return (matcherInput) => !!expressionToNegate && !expressionToNegate(matcherInput);
    }
    if (token2 === "(") {
      token2 = tokenizer.next();
      const expressionInParents = parseInnerExpression();
      if (token2 === ")") {
        token2 = tokenizer.next();
      }
      return expressionInParents;
    }
    if (isIdentifier(token2)) {
      const identifiers = [];
      do {
        identifiers.push(token2);
        token2 = tokenizer.next();
      } while (isIdentifier(token2));
      return (matcherInput) => matchesName(identifiers, matcherInput);
    }
    return null;
  }
  function parseConjunction() {
    const matchers = [];
    let matcher = parseOperand();
    while (matcher) {
      matchers.push(matcher);
      matcher = parseOperand();
    }
    return (matcherInput) => matchers.every((matcher2) => matcher2(matcherInput));
  }
  function parseInnerExpression() {
    const matchers = [];
    let matcher = parseConjunction();
    while (matcher) {
      matchers.push(matcher);
      if (token2 === "|" || token2 === ",") {
        do {
          token2 = tokenizer.next();
        } while (token2 === "|" || token2 === ",");
      } else {
        break;
      }
      matcher = parseConjunction();
    }
    return (matcherInput) => matchers.some((matcher2) => matcher2(matcherInput));
  }
}
function isIdentifier(token2) {
  return !!token2 && !!token2.match(/[\w\.:]+/);
}
function newTokenizer(input) {
  let regex = /([LR]:|[\w\.:][\w\.:\-]*|[\,\|\-\(\)])/g;
  let match = regex.exec(input);
  return {
    next: () => {
      if (!match) {
        return null;
      }
      const res = match[0];
      match = regex.exec(input);
      return res;
    }
  };
}
function disposeOnigString(str) {
  if (typeof str.dispose === "function") {
    str.dispose();
  }
}
var TopLevelRuleReference = class {
  constructor(scopeName) {
    this.scopeName = scopeName;
  }
  toKey() {
    return this.scopeName;
  }
};
var TopLevelRepositoryRuleReference = class {
  constructor(scopeName, ruleName) {
    this.scopeName = scopeName;
    this.ruleName = ruleName;
  }
  toKey() {
    return `${this.scopeName}#${this.ruleName}`;
  }
};
var ExternalReferenceCollector = class {
  constructor() {
    __publicField(this, "_references", []);
    __publicField(this, "_seenReferenceKeys", /* @__PURE__ */ new Set());
    __publicField(this, "visitedRule", /* @__PURE__ */ new Set());
  }
  get references() {
    return this._references;
  }
  add(reference) {
    const key2 = reference.toKey();
    if (this._seenReferenceKeys.has(key2)) {
      return;
    }
    this._seenReferenceKeys.add(key2);
    this._references.push(reference);
  }
};
var ScopeDependencyProcessor = class {
  constructor(repo, initialScopeName) {
    __publicField(this, "seenFullScopeRequests", /* @__PURE__ */ new Set());
    __publicField(this, "seenPartialScopeRequests", /* @__PURE__ */ new Set());
    __publicField(this, "Q");
    this.repo = repo;
    this.initialScopeName = initialScopeName;
    this.seenFullScopeRequests.add(this.initialScopeName);
    this.Q = [new TopLevelRuleReference(this.initialScopeName)];
  }
  processQueue() {
    const q2 = this.Q;
    this.Q = [];
    const deps = new ExternalReferenceCollector();
    for (const dep of q2) {
      collectReferencesOfReference(dep, this.initialScopeName, this.repo, deps);
    }
    for (const dep of deps.references) {
      if (dep instanceof TopLevelRuleReference) {
        if (this.seenFullScopeRequests.has(dep.scopeName)) {
          continue;
        }
        this.seenFullScopeRequests.add(dep.scopeName);
        this.Q.push(dep);
      } else {
        if (this.seenFullScopeRequests.has(dep.scopeName)) {
          continue;
        }
        if (this.seenPartialScopeRequests.has(dep.toKey())) {
          continue;
        }
        this.seenPartialScopeRequests.add(dep.toKey());
        this.Q.push(dep);
      }
    }
  }
};
function collectReferencesOfReference(reference, baseGrammarScopeName, repo, result) {
  const selfGrammar = repo.lookup(reference.scopeName);
  if (!selfGrammar) {
    if (reference.scopeName === baseGrammarScopeName) {
      throw new Error(`No grammar provided for <${baseGrammarScopeName}>`);
    }
    return;
  }
  const baseGrammar = repo.lookup(baseGrammarScopeName);
  if (reference instanceof TopLevelRuleReference) {
    collectExternalReferencesInTopLevelRule({ baseGrammar, selfGrammar }, result);
  } else {
    collectExternalReferencesInTopLevelRepositoryRule(
      reference.ruleName,
      { baseGrammar, selfGrammar, repository: selfGrammar.repository },
      result
    );
  }
  const injections = repo.injections(reference.scopeName);
  if (injections) {
    for (const injection of injections) {
      result.add(new TopLevelRuleReference(injection));
    }
  }
}
function collectExternalReferencesInTopLevelRepositoryRule(ruleName, context, result) {
  if (context.repository && context.repository[ruleName]) {
    const rule = context.repository[ruleName];
    collectExternalReferencesInRules([rule], context, result);
  }
}
function collectExternalReferencesInTopLevelRule(context, result) {
  if (context.selfGrammar.patterns && Array.isArray(context.selfGrammar.patterns)) {
    collectExternalReferencesInRules(
      context.selfGrammar.patterns,
      { ...context, repository: context.selfGrammar.repository },
      result
    );
  }
  if (context.selfGrammar.injections) {
    collectExternalReferencesInRules(
      Object.values(context.selfGrammar.injections),
      { ...context, repository: context.selfGrammar.repository },
      result
    );
  }
}
function collectExternalReferencesInRules(rules, context, result) {
  for (const rule of rules) {
    if (result.visitedRule.has(rule)) {
      continue;
    }
    result.visitedRule.add(rule);
    const patternRepository = rule.repository ? mergeObjects({}, context.repository, rule.repository) : context.repository;
    if (Array.isArray(rule.patterns)) {
      collectExternalReferencesInRules(rule.patterns, { ...context, repository: patternRepository }, result);
    }
    const include = rule.include;
    if (!include) {
      continue;
    }
    const reference = parseInclude(include);
    switch (reference.kind) {
      case 0:
        collectExternalReferencesInTopLevelRule({ ...context, selfGrammar: context.baseGrammar }, result);
        break;
      case 1:
        collectExternalReferencesInTopLevelRule(context, result);
        break;
      case 2:
        collectExternalReferencesInTopLevelRepositoryRule(reference.ruleName, { ...context, repository: patternRepository }, result);
        break;
      case 3:
      case 4:
        const selfGrammar = reference.scopeName === context.selfGrammar.scopeName ? context.selfGrammar : reference.scopeName === context.baseGrammar.scopeName ? context.baseGrammar : void 0;
        if (selfGrammar) {
          const newContext = { baseGrammar: context.baseGrammar, selfGrammar, repository: patternRepository };
          if (reference.kind === 4) {
            collectExternalReferencesInTopLevelRepositoryRule(reference.ruleName, newContext, result);
          } else {
            collectExternalReferencesInTopLevelRule(newContext, result);
          }
        } else {
          if (reference.kind === 4) {
            result.add(new TopLevelRepositoryRuleReference(reference.scopeName, reference.ruleName));
          } else {
            result.add(new TopLevelRuleReference(reference.scopeName));
          }
        }
        break;
    }
  }
}
var BaseReference = class {
  constructor() {
    __publicField(this, "kind", 0);
  }
};
var SelfReference = class {
  constructor() {
    __publicField(this, "kind", 1);
  }
};
var RelativeReference = class {
  constructor(ruleName) {
    __publicField(this, "kind", 2);
    this.ruleName = ruleName;
  }
};
var TopLevelReference = class {
  constructor(scopeName) {
    __publicField(this, "kind", 3);
    this.scopeName = scopeName;
  }
};
var TopLevelRepositoryReference = class {
  constructor(scopeName, ruleName) {
    __publicField(this, "kind", 4);
    this.scopeName = scopeName;
    this.ruleName = ruleName;
  }
};
function parseInclude(include) {
  if (include === "$base") {
    return new BaseReference();
  } else if (include === "$self") {
    return new SelfReference();
  }
  const indexOfSharp = include.indexOf("#");
  if (indexOfSharp === -1) {
    return new TopLevelReference(include);
  } else if (indexOfSharp === 0) {
    return new RelativeReference(include.substring(1));
  } else {
    const scopeName = include.substring(0, indexOfSharp);
    const ruleName = include.substring(indexOfSharp + 1);
    return new TopLevelRepositoryReference(scopeName, ruleName);
  }
}
var HAS_BACK_REFERENCES = /\\(\d+)/;
var BACK_REFERENCING_END = /\\(\d+)/g;
var endRuleId = -1;
var whileRuleId = -2;
function ruleIdFromNumber(id) {
  return id;
}
function ruleIdToNumber(id) {
  return id;
}
var Rule = class {
  constructor($location, id, name, contentName) {
    __publicField(this, "$location");
    __publicField(this, "id");
    __publicField(this, "_nameIsCapturing");
    __publicField(this, "_name");
    __publicField(this, "_contentNameIsCapturing");
    __publicField(this, "_contentName");
    this.$location = $location;
    this.id = id;
    this._name = name || null;
    this._nameIsCapturing = RegexSource.hasCaptures(this._name);
    this._contentName = contentName || null;
    this._contentNameIsCapturing = RegexSource.hasCaptures(this._contentName);
  }
  get debugName() {
    const location = this.$location ? `${basename(this.$location.filename)}:${this.$location.line}` : "unknown";
    return `${this.constructor.name}#${this.id} @ ${location}`;
  }
  getName(lineText, captureIndices) {
    if (!this._nameIsCapturing || this._name === null || lineText === null || captureIndices === null) {
      return this._name;
    }
    return RegexSource.replaceCaptures(this._name, lineText, captureIndices);
  }
  getContentName(lineText, captureIndices) {
    if (!this._contentNameIsCapturing || this._contentName === null) {
      return this._contentName;
    }
    return RegexSource.replaceCaptures(this._contentName, lineText, captureIndices);
  }
};
var CaptureRule = class extends Rule {
  constructor($location, id, name, contentName, retokenizeCapturedWithRuleId) {
    super($location, id, name, contentName);
    __publicField(this, "retokenizeCapturedWithRuleId");
    this.retokenizeCapturedWithRuleId = retokenizeCapturedWithRuleId;
  }
  dispose() {
  }
  collectPatterns(grammar, out) {
    throw new Error("Not supported!");
  }
  compile(grammar, endRegexSource) {
    throw new Error("Not supported!");
  }
  compileAG(grammar, endRegexSource, allowA, allowG) {
    throw new Error("Not supported!");
  }
};
var MatchRule = class extends Rule {
  constructor($location, id, name, match, captures) {
    super($location, id, name, null);
    __publicField(this, "_match");
    __publicField(this, "captures");
    __publicField(this, "_cachedCompiledPatterns");
    this._match = new RegExpSource(match, this.id);
    this.captures = captures;
    this._cachedCompiledPatterns = null;
  }
  dispose() {
    if (this._cachedCompiledPatterns) {
      this._cachedCompiledPatterns.dispose();
      this._cachedCompiledPatterns = null;
    }
  }
  get debugMatchRegExp() {
    return `${this._match.source}`;
  }
  collectPatterns(grammar, out) {
    out.push(this._match);
  }
  compile(grammar, endRegexSource) {
    return this._getCachedCompiledPatterns(grammar).compile(grammar);
  }
  compileAG(grammar, endRegexSource, allowA, allowG) {
    return this._getCachedCompiledPatterns(grammar).compileAG(grammar, allowA, allowG);
  }
  _getCachedCompiledPatterns(grammar) {
    if (!this._cachedCompiledPatterns) {
      this._cachedCompiledPatterns = new RegExpSourceList();
      this.collectPatterns(grammar, this._cachedCompiledPatterns);
    }
    return this._cachedCompiledPatterns;
  }
};
var IncludeOnlyRule = class extends Rule {
  constructor($location, id, name, contentName, patterns) {
    super($location, id, name, contentName);
    __publicField(this, "hasMissingPatterns");
    __publicField(this, "patterns");
    __publicField(this, "_cachedCompiledPatterns");
    this.patterns = patterns.patterns;
    this.hasMissingPatterns = patterns.hasMissingPatterns;
    this._cachedCompiledPatterns = null;
  }
  dispose() {
    if (this._cachedCompiledPatterns) {
      this._cachedCompiledPatterns.dispose();
      this._cachedCompiledPatterns = null;
    }
  }
  collectPatterns(grammar, out) {
    for (const pattern of this.patterns) {
      const rule = grammar.getRule(pattern);
      rule.collectPatterns(grammar, out);
    }
  }
  compile(grammar, endRegexSource) {
    return this._getCachedCompiledPatterns(grammar).compile(grammar);
  }
  compileAG(grammar, endRegexSource, allowA, allowG) {
    return this._getCachedCompiledPatterns(grammar).compileAG(grammar, allowA, allowG);
  }
  _getCachedCompiledPatterns(grammar) {
    if (!this._cachedCompiledPatterns) {
      this._cachedCompiledPatterns = new RegExpSourceList();
      this.collectPatterns(grammar, this._cachedCompiledPatterns);
    }
    return this._cachedCompiledPatterns;
  }
};
var BeginEndRule = class extends Rule {
  constructor($location, id, name, contentName, begin, beginCaptures, end, endCaptures, applyEndPatternLast, patterns) {
    super($location, id, name, contentName);
    __publicField(this, "_begin");
    __publicField(this, "beginCaptures");
    __publicField(this, "_end");
    __publicField(this, "endHasBackReferences");
    __publicField(this, "endCaptures");
    __publicField(this, "applyEndPatternLast");
    __publicField(this, "hasMissingPatterns");
    __publicField(this, "patterns");
    __publicField(this, "_cachedCompiledPatterns");
    this._begin = new RegExpSource(begin, this.id);
    this.beginCaptures = beginCaptures;
    this._end = new RegExpSource(end ? end : "￿", -1);
    this.endHasBackReferences = this._end.hasBackReferences;
    this.endCaptures = endCaptures;
    this.applyEndPatternLast = applyEndPatternLast || false;
    this.patterns = patterns.patterns;
    this.hasMissingPatterns = patterns.hasMissingPatterns;
    this._cachedCompiledPatterns = null;
  }
  dispose() {
    if (this._cachedCompiledPatterns) {
      this._cachedCompiledPatterns.dispose();
      this._cachedCompiledPatterns = null;
    }
  }
  get debugBeginRegExp() {
    return `${this._begin.source}`;
  }
  get debugEndRegExp() {
    return `${this._end.source}`;
  }
  getEndWithResolvedBackReferences(lineText, captureIndices) {
    return this._end.resolveBackReferences(lineText, captureIndices);
  }
  collectPatterns(grammar, out) {
    out.push(this._begin);
  }
  compile(grammar, endRegexSource) {
    return this._getCachedCompiledPatterns(grammar, endRegexSource).compile(grammar);
  }
  compileAG(grammar, endRegexSource, allowA, allowG) {
    return this._getCachedCompiledPatterns(grammar, endRegexSource).compileAG(grammar, allowA, allowG);
  }
  _getCachedCompiledPatterns(grammar, endRegexSource) {
    if (!this._cachedCompiledPatterns) {
      this._cachedCompiledPatterns = new RegExpSourceList();
      for (const pattern of this.patterns) {
        const rule = grammar.getRule(pattern);
        rule.collectPatterns(grammar, this._cachedCompiledPatterns);
      }
      if (this.applyEndPatternLast) {
        this._cachedCompiledPatterns.push(this._end.hasBackReferences ? this._end.clone() : this._end);
      } else {
        this._cachedCompiledPatterns.unshift(this._end.hasBackReferences ? this._end.clone() : this._end);
      }
    }
    if (this._end.hasBackReferences) {
      if (this.applyEndPatternLast) {
        this._cachedCompiledPatterns.setSource(this._cachedCompiledPatterns.length() - 1, endRegexSource);
      } else {
        this._cachedCompiledPatterns.setSource(0, endRegexSource);
      }
    }
    return this._cachedCompiledPatterns;
  }
};
var BeginWhileRule = class extends Rule {
  constructor($location, id, name, contentName, begin, beginCaptures, _while, whileCaptures, patterns) {
    super($location, id, name, contentName);
    __publicField(this, "_begin");
    __publicField(this, "beginCaptures");
    __publicField(this, "whileCaptures");
    __publicField(this, "_while");
    __publicField(this, "whileHasBackReferences");
    __publicField(this, "hasMissingPatterns");
    __publicField(this, "patterns");
    __publicField(this, "_cachedCompiledPatterns");
    __publicField(this, "_cachedCompiledWhilePatterns");
    this._begin = new RegExpSource(begin, this.id);
    this.beginCaptures = beginCaptures;
    this.whileCaptures = whileCaptures;
    this._while = new RegExpSource(_while, whileRuleId);
    this.whileHasBackReferences = this._while.hasBackReferences;
    this.patterns = patterns.patterns;
    this.hasMissingPatterns = patterns.hasMissingPatterns;
    this._cachedCompiledPatterns = null;
    this._cachedCompiledWhilePatterns = null;
  }
  dispose() {
    if (this._cachedCompiledPatterns) {
      this._cachedCompiledPatterns.dispose();
      this._cachedCompiledPatterns = null;
    }
    if (this._cachedCompiledWhilePatterns) {
      this._cachedCompiledWhilePatterns.dispose();
      this._cachedCompiledWhilePatterns = null;
    }
  }
  get debugBeginRegExp() {
    return `${this._begin.source}`;
  }
  get debugWhileRegExp() {
    return `${this._while.source}`;
  }
  getWhileWithResolvedBackReferences(lineText, captureIndices) {
    return this._while.resolveBackReferences(lineText, captureIndices);
  }
  collectPatterns(grammar, out) {
    out.push(this._begin);
  }
  compile(grammar, endRegexSource) {
    return this._getCachedCompiledPatterns(grammar).compile(grammar);
  }
  compileAG(grammar, endRegexSource, allowA, allowG) {
    return this._getCachedCompiledPatterns(grammar).compileAG(grammar, allowA, allowG);
  }
  _getCachedCompiledPatterns(grammar) {
    if (!this._cachedCompiledPatterns) {
      this._cachedCompiledPatterns = new RegExpSourceList();
      for (const pattern of this.patterns) {
        const rule = grammar.getRule(pattern);
        rule.collectPatterns(grammar, this._cachedCompiledPatterns);
      }
    }
    return this._cachedCompiledPatterns;
  }
  compileWhile(grammar, endRegexSource) {
    return this._getCachedCompiledWhilePatterns(grammar, endRegexSource).compile(grammar);
  }
  compileWhileAG(grammar, endRegexSource, allowA, allowG) {
    return this._getCachedCompiledWhilePatterns(grammar, endRegexSource).compileAG(grammar, allowA, allowG);
  }
  _getCachedCompiledWhilePatterns(grammar, endRegexSource) {
    if (!this._cachedCompiledWhilePatterns) {
      this._cachedCompiledWhilePatterns = new RegExpSourceList();
      this._cachedCompiledWhilePatterns.push(this._while.hasBackReferences ? this._while.clone() : this._while);
    }
    if (this._while.hasBackReferences) {
      this._cachedCompiledWhilePatterns.setSource(0, endRegexSource ? endRegexSource : "￿");
    }
    return this._cachedCompiledWhilePatterns;
  }
};
var RuleFactory = class _RuleFactory {
  static createCaptureRule(helper, $location, name, contentName, retokenizeCapturedWithRuleId) {
    return helper.registerRule((id) => {
      return new CaptureRule($location, id, name, contentName, retokenizeCapturedWithRuleId);
    });
  }
  static getCompiledRuleId(desc, helper, repository) {
    if (!desc.id) {
      helper.registerRule((id) => {
        desc.id = id;
        if (desc.match) {
          return new MatchRule(
            desc.$vscodeTextmateLocation,
            desc.id,
            desc.name,
            desc.match,
            _RuleFactory._compileCaptures(desc.captures, helper, repository)
          );
        }
        if (typeof desc.begin === "undefined") {
          if (desc.repository) {
            repository = mergeObjects({}, repository, desc.repository);
          }
          let patterns = desc.patterns;
          if (typeof patterns === "undefined" && desc.include) {
            patterns = [{ include: desc.include }];
          }
          return new IncludeOnlyRule(
            desc.$vscodeTextmateLocation,
            desc.id,
            desc.name,
            desc.contentName,
            _RuleFactory._compilePatterns(patterns, helper, repository)
          );
        }
        if (desc.while) {
          return new BeginWhileRule(
            desc.$vscodeTextmateLocation,
            desc.id,
            desc.name,
            desc.contentName,
            desc.begin,
            _RuleFactory._compileCaptures(desc.beginCaptures || desc.captures, helper, repository),
            desc.while,
            _RuleFactory._compileCaptures(desc.whileCaptures || desc.captures, helper, repository),
            _RuleFactory._compilePatterns(desc.patterns, helper, repository)
          );
        }
        return new BeginEndRule(
          desc.$vscodeTextmateLocation,
          desc.id,
          desc.name,
          desc.contentName,
          desc.begin,
          _RuleFactory._compileCaptures(desc.beginCaptures || desc.captures, helper, repository),
          desc.end,
          _RuleFactory._compileCaptures(desc.endCaptures || desc.captures, helper, repository),
          desc.applyEndPatternLast,
          _RuleFactory._compilePatterns(desc.patterns, helper, repository)
        );
      });
    }
    return desc.id;
  }
  static _compileCaptures(captures, helper, repository) {
    let r2 = [];
    if (captures) {
      let maximumCaptureId = 0;
      for (const captureId in captures) {
        if (captureId === "$vscodeTextmateLocation") {
          continue;
        }
        const numericCaptureId = parseInt(captureId, 10);
        if (numericCaptureId > maximumCaptureId) {
          maximumCaptureId = numericCaptureId;
        }
      }
      for (let i2 = 0; i2 <= maximumCaptureId; i2++) {
        r2[i2] = null;
      }
      for (const captureId in captures) {
        if (captureId === "$vscodeTextmateLocation") {
          continue;
        }
        const numericCaptureId = parseInt(captureId, 10);
        let retokenizeCapturedWithRuleId = 0;
        if (captures[captureId].patterns) {
          retokenizeCapturedWithRuleId = _RuleFactory.getCompiledRuleId(captures[captureId], helper, repository);
        }
        r2[numericCaptureId] = _RuleFactory.createCaptureRule(helper, captures[captureId].$vscodeTextmateLocation, captures[captureId].name, captures[captureId].contentName, retokenizeCapturedWithRuleId);
      }
    }
    return r2;
  }
  static _compilePatterns(patterns, helper, repository) {
    let r2 = [];
    if (patterns) {
      for (let i2 = 0, len = patterns.length; i2 < len; i2++) {
        const pattern = patterns[i2];
        let ruleId = -1;
        if (pattern.include) {
          const reference = parseInclude(pattern.include);
          switch (reference.kind) {
            case 0:
            case 1:
              ruleId = _RuleFactory.getCompiledRuleId(repository[pattern.include], helper, repository);
              break;
            case 2:
              let localIncludedRule = repository[reference.ruleName];
              if (localIncludedRule) {
                ruleId = _RuleFactory.getCompiledRuleId(localIncludedRule, helper, repository);
              }
              break;
            case 3:
            case 4:
              const externalGrammarName = reference.scopeName;
              const externalGrammarInclude = reference.kind === 4 ? reference.ruleName : null;
              const externalGrammar = helper.getExternalGrammar(externalGrammarName, repository);
              if (externalGrammar) {
                if (externalGrammarInclude) {
                  let externalIncludedRule = externalGrammar.repository[externalGrammarInclude];
                  if (externalIncludedRule) {
                    ruleId = _RuleFactory.getCompiledRuleId(externalIncludedRule, helper, externalGrammar.repository);
                  }
                } else {
                  ruleId = _RuleFactory.getCompiledRuleId(externalGrammar.repository.$self, helper, externalGrammar.repository);
                }
              }
              break;
          }
        } else {
          ruleId = _RuleFactory.getCompiledRuleId(pattern, helper, repository);
        }
        if (ruleId !== -1) {
          const rule = helper.getRule(ruleId);
          let skipRule = false;
          if (rule instanceof IncludeOnlyRule || rule instanceof BeginEndRule || rule instanceof BeginWhileRule) {
            if (rule.hasMissingPatterns && rule.patterns.length === 0) {
              skipRule = true;
            }
          }
          if (skipRule) {
            continue;
          }
          r2.push(ruleId);
        }
      }
    }
    return {
      patterns: r2,
      hasMissingPatterns: (patterns ? patterns.length : 0) !== r2.length
    };
  }
};
var RegExpSource = class _RegExpSource {
  constructor(regExpSource, ruleId) {
    __publicField(this, "source");
    __publicField(this, "ruleId");
    __publicField(this, "hasAnchor");
    __publicField(this, "hasBackReferences");
    __publicField(this, "_anchorCache");
    if (regExpSource && typeof regExpSource === "string") {
      const len = regExpSource.length;
      let lastPushedPos = 0;
      let output = [];
      let hasAnchor = false;
      for (let pos = 0; pos < len; pos++) {
        const ch = regExpSource.charAt(pos);
        if (ch === "\\") {
          if (pos + 1 < len) {
            const nextCh = regExpSource.charAt(pos + 1);
            if (nextCh === "z") {
              output.push(regExpSource.substring(lastPushedPos, pos));
              output.push("$(?!\\n)(?<!\\n)");
              lastPushedPos = pos + 2;
            } else if (nextCh === "A" || nextCh === "G") {
              hasAnchor = true;
            }
            pos++;
          }
        }
      }
      this.hasAnchor = hasAnchor;
      if (lastPushedPos === 0) {
        this.source = regExpSource;
      } else {
        output.push(regExpSource.substring(lastPushedPos, len));
        this.source = output.join("");
      }
    } else {
      this.hasAnchor = false;
      this.source = regExpSource;
    }
    if (this.hasAnchor) {
      this._anchorCache = this._buildAnchorCache();
    } else {
      this._anchorCache = null;
    }
    this.ruleId = ruleId;
    if (typeof this.source === "string") {
      this.hasBackReferences = HAS_BACK_REFERENCES.test(this.source);
    } else {
      this.hasBackReferences = false;
    }
  }
  clone() {
    return new _RegExpSource(this.source, this.ruleId);
  }
  setSource(newSource) {
    if (this.source === newSource) {
      return;
    }
    this.source = newSource;
    if (this.hasAnchor) {
      this._anchorCache = this._buildAnchorCache();
    }
  }
  resolveBackReferences(lineText, captureIndices) {
    if (typeof this.source !== "string") {
      throw new Error("This method should only be called if the source is a string");
    }
    let capturedValues = captureIndices.map((capture) => {
      return lineText.substring(capture.start, capture.end);
    });
    BACK_REFERENCING_END.lastIndex = 0;
    return this.source.replace(BACK_REFERENCING_END, (match, g1) => {
      return escapeRegExpCharacters(capturedValues[parseInt(g1, 10)] || "");
    });
  }
  _buildAnchorCache() {
    if (typeof this.source !== "string") {
      throw new Error("This method should only be called if the source is a string");
    }
    let A0_G0_result = [];
    let A0_G1_result = [];
    let A1_G0_result = [];
    let A1_G1_result = [];
    let pos, len, ch, nextCh;
    for (pos = 0, len = this.source.length; pos < len; pos++) {
      ch = this.source.charAt(pos);
      A0_G0_result[pos] = ch;
      A0_G1_result[pos] = ch;
      A1_G0_result[pos] = ch;
      A1_G1_result[pos] = ch;
      if (ch === "\\") {
        if (pos + 1 < len) {
          nextCh = this.source.charAt(pos + 1);
          if (nextCh === "A") {
            A0_G0_result[pos + 1] = "￿";
            A0_G1_result[pos + 1] = "￿";
            A1_G0_result[pos + 1] = "A";
            A1_G1_result[pos + 1] = "A";
          } else if (nextCh === "G") {
            A0_G0_result[pos + 1] = "￿";
            A0_G1_result[pos + 1] = "G";
            A1_G0_result[pos + 1] = "￿";
            A1_G1_result[pos + 1] = "G";
          } else {
            A0_G0_result[pos + 1] = nextCh;
            A0_G1_result[pos + 1] = nextCh;
            A1_G0_result[pos + 1] = nextCh;
            A1_G1_result[pos + 1] = nextCh;
          }
          pos++;
        }
      }
    }
    return {
      A0_G0: A0_G0_result.join(""),
      A0_G1: A0_G1_result.join(""),
      A1_G0: A1_G0_result.join(""),
      A1_G1: A1_G1_result.join("")
    };
  }
  resolveAnchors(allowA, allowG) {
    if (!this.hasAnchor || !this._anchorCache || typeof this.source !== "string") {
      return this.source;
    }
    if (allowA) {
      if (allowG) {
        return this._anchorCache.A1_G1;
      } else {
        return this._anchorCache.A1_G0;
      }
    } else {
      if (allowG) {
        return this._anchorCache.A0_G1;
      } else {
        return this._anchorCache.A0_G0;
      }
    }
  }
};
var RegExpSourceList = class {
  constructor() {
    __publicField(this, "_items");
    __publicField(this, "_hasAnchors");
    __publicField(this, "_cached");
    __publicField(this, "_anchorCache");
    this._items = [];
    this._hasAnchors = false;
    this._cached = null;
    this._anchorCache = {
      A0_G0: null,
      A0_G1: null,
      A1_G0: null,
      A1_G1: null
    };
  }
  dispose() {
    this._disposeCaches();
  }
  _disposeCaches() {
    if (this._cached) {
      this._cached.dispose();
      this._cached = null;
    }
    if (this._anchorCache.A0_G0) {
      this._anchorCache.A0_G0.dispose();
      this._anchorCache.A0_G0 = null;
    }
    if (this._anchorCache.A0_G1) {
      this._anchorCache.A0_G1.dispose();
      this._anchorCache.A0_G1 = null;
    }
    if (this._anchorCache.A1_G0) {
      this._anchorCache.A1_G0.dispose();
      this._anchorCache.A1_G0 = null;
    }
    if (this._anchorCache.A1_G1) {
      this._anchorCache.A1_G1.dispose();
      this._anchorCache.A1_G1 = null;
    }
  }
  push(item) {
    this._items.push(item);
    this._hasAnchors = this._hasAnchors || item.hasAnchor;
  }
  unshift(item) {
    this._items.unshift(item);
    this._hasAnchors = this._hasAnchors || item.hasAnchor;
  }
  length() {
    return this._items.length;
  }
  setSource(index, newSource) {
    if (this._items[index].source !== newSource) {
      this._disposeCaches();
      this._items[index].setSource(newSource);
    }
  }
  compile(onigLib) {
    if (!this._cached) {
      let regExps = this._items.map((e) => e.source);
      this._cached = new CompiledRule(onigLib, regExps, this._items.map((e) => e.ruleId));
    }
    return this._cached;
  }
  compileAG(onigLib, allowA, allowG) {
    if (!this._hasAnchors) {
      return this.compile(onigLib);
    } else {
      if (allowA) {
        if (allowG) {
          if (!this._anchorCache.A1_G1) {
            this._anchorCache.A1_G1 = this._resolveAnchors(onigLib, allowA, allowG);
          }
          return this._anchorCache.A1_G1;
        } else {
          if (!this._anchorCache.A1_G0) {
            this._anchorCache.A1_G0 = this._resolveAnchors(onigLib, allowA, allowG);
          }
          return this._anchorCache.A1_G0;
        }
      } else {
        if (allowG) {
          if (!this._anchorCache.A0_G1) {
            this._anchorCache.A0_G1 = this._resolveAnchors(onigLib, allowA, allowG);
          }
          return this._anchorCache.A0_G1;
        } else {
          if (!this._anchorCache.A0_G0) {
            this._anchorCache.A0_G0 = this._resolveAnchors(onigLib, allowA, allowG);
          }
          return this._anchorCache.A0_G0;
        }
      }
    }
  }
  _resolveAnchors(onigLib, allowA, allowG) {
    let regExps = this._items.map((e) => e.resolveAnchors(allowA, allowG));
    return new CompiledRule(onigLib, regExps, this._items.map((e) => e.ruleId));
  }
};
var CompiledRule = class {
  constructor(onigLib, regExps, rules) {
    __publicField(this, "scanner");
    this.regExps = regExps;
    this.rules = rules;
    this.scanner = onigLib.createOnigScanner(regExps);
  }
  dispose() {
    if (typeof this.scanner.dispose === "function") {
      this.scanner.dispose();
    }
  }
  toString() {
    const r2 = [];
    for (let i2 = 0, len = this.rules.length; i2 < len; i2++) {
      r2.push("   - " + this.rules[i2] + ": " + this.regExps[i2]);
    }
    return r2.join("\n");
  }
  findNextMatchSync(string, startPosition, options) {
    const result = this.scanner.findNextMatchSync(string, startPosition, options);
    if (!result) {
      return null;
    }
    return {
      ruleId: this.rules[result.index],
      captureIndices: result.captureIndices
    };
  }
};
var BasicScopeAttributes = class {
  constructor(languageId, tokenType) {
    this.languageId = languageId;
    this.tokenType = tokenType;
  }
};
var BasicScopeAttributesProvider = (_a = class {
  constructor(initialLanguageId, embeddedLanguages) {
    __publicField(this, "_defaultAttributes");
    __publicField(this, "_embeddedLanguagesMatcher");
    __publicField(this, "_getBasicScopeAttributes", new CachedFn((scopeName) => {
      const languageId = this._scopeToLanguage(scopeName);
      const standardTokenType = this._toStandardTokenType(scopeName);
      return new BasicScopeAttributes(languageId, standardTokenType);
    }));
    this._defaultAttributes = new BasicScopeAttributes(
      initialLanguageId,
      8
      /* NotSet */
    );
    this._embeddedLanguagesMatcher = new ScopeMatcher(Object.entries(embeddedLanguages || {}));
  }
  getDefaultAttributes() {
    return this._defaultAttributes;
  }
  getBasicScopeAttributes(scopeName) {
    if (scopeName === null) {
      return _a._NULL_SCOPE_METADATA;
    }
    return this._getBasicScopeAttributes.get(scopeName);
  }
  /**
   * Given a produced TM scope, return the language that token describes or null if unknown.
   * e.g. source.html => html, source.css.embedded.html => css, punctuation.definition.tag.html => null
   */
  _scopeToLanguage(scope) {
    return this._embeddedLanguagesMatcher.match(scope) || 0;
  }
  _toStandardTokenType(scopeName) {
    const m2 = scopeName.match(_a.STANDARD_TOKEN_TYPE_REGEXP);
    if (!m2) {
      return 8;
    }
    switch (m2[1]) {
      case "comment":
        return 1;
      case "string":
        return 2;
      case "regex":
        return 3;
      case "meta.embedded":
        return 0;
    }
    throw new Error("Unexpected match for standard token type!");
  }
}, __publicField(_a, "_NULL_SCOPE_METADATA", new BasicScopeAttributes(0, 0)), __publicField(_a, "STANDARD_TOKEN_TYPE_REGEXP", /\b(comment|string|regex|meta\.embedded)\b/), _a);
var ScopeMatcher = class {
  constructor(values) {
    __publicField(this, "values");
    __publicField(this, "scopesRegExp");
    if (values.length === 0) {
      this.values = null;
      this.scopesRegExp = null;
    } else {
      this.values = new Map(values);
      const escapedScopes = values.map(
        ([scopeName, value]) => escapeRegExpCharacters(scopeName)
      );
      escapedScopes.sort();
      escapedScopes.reverse();
      this.scopesRegExp = new RegExp(
        `^((${escapedScopes.join(")|(")}))($|\\.)`,
        ""
      );
    }
  }
  match(scope) {
    if (!this.scopesRegExp) {
      return void 0;
    }
    const m2 = scope.match(this.scopesRegExp);
    if (!m2) {
      return void 0;
    }
    return this.values.get(m2[1]);
  }
};
var TokenizeStringResult = class {
  constructor(stack, stoppedEarly) {
    this.stack = stack;
    this.stoppedEarly = stoppedEarly;
  }
};
function _tokenizeString(grammar, lineText, isFirstLine, linePos, stack, lineTokens, checkWhileConditions, timeLimit) {
  const lineLength = lineText.content.length;
  let STOP = false;
  let anchorPosition = -1;
  if (checkWhileConditions) {
    const whileCheckResult = _checkWhileConditions(
      grammar,
      lineText,
      isFirstLine,
      linePos,
      stack,
      lineTokens
    );
    stack = whileCheckResult.stack;
    linePos = whileCheckResult.linePos;
    isFirstLine = whileCheckResult.isFirstLine;
    anchorPosition = whileCheckResult.anchorPosition;
  }
  const startTime = Date.now();
  while (!STOP) {
    if (timeLimit !== 0) {
      const elapsedTime = Date.now() - startTime;
      if (elapsedTime > timeLimit) {
        return new TokenizeStringResult(stack, true);
      }
    }
    scanNext();
  }
  return new TokenizeStringResult(stack, false);
  function scanNext() {
    const r2 = matchRuleOrInjections(
      grammar,
      lineText,
      isFirstLine,
      linePos,
      stack,
      anchorPosition
    );
    if (!r2) {
      lineTokens.produce(stack, lineLength);
      STOP = true;
      return;
    }
    const captureIndices = r2.captureIndices;
    const matchedRuleId = r2.matchedRuleId;
    const hasAdvanced = captureIndices && captureIndices.length > 0 ? captureIndices[0].end > linePos : false;
    if (matchedRuleId === endRuleId) {
      const poppedRule = stack.getRule(grammar);
      lineTokens.produce(stack, captureIndices[0].start);
      stack = stack.withContentNameScopesList(stack.nameScopesList);
      handleCaptures(
        grammar,
        lineText,
        isFirstLine,
        stack,
        lineTokens,
        poppedRule.endCaptures,
        captureIndices
      );
      lineTokens.produce(stack, captureIndices[0].end);
      const popped = stack;
      stack = stack.parent;
      anchorPosition = popped.getAnchorPos();
      if (!hasAdvanced && popped.getEnterPos() === linePos) {
        stack = popped;
        lineTokens.produce(stack, lineLength);
        STOP = true;
        return;
      }
    } else {
      const _rule = grammar.getRule(matchedRuleId);
      lineTokens.produce(stack, captureIndices[0].start);
      const beforePush = stack;
      const scopeName = _rule.getName(lineText.content, captureIndices);
      const nameScopesList = stack.contentNameScopesList.pushAttributed(
        scopeName,
        grammar
      );
      stack = stack.push(
        matchedRuleId,
        linePos,
        anchorPosition,
        captureIndices[0].end === lineLength,
        null,
        nameScopesList,
        nameScopesList
      );
      if (_rule instanceof BeginEndRule) {
        const pushedRule = _rule;
        handleCaptures(
          grammar,
          lineText,
          isFirstLine,
          stack,
          lineTokens,
          pushedRule.beginCaptures,
          captureIndices
        );
        lineTokens.produce(stack, captureIndices[0].end);
        anchorPosition = captureIndices[0].end;
        const contentName = pushedRule.getContentName(
          lineText.content,
          captureIndices
        );
        const contentNameScopesList = nameScopesList.pushAttributed(
          contentName,
          grammar
        );
        stack = stack.withContentNameScopesList(contentNameScopesList);
        if (pushedRule.endHasBackReferences) {
          stack = stack.withEndRule(
            pushedRule.getEndWithResolvedBackReferences(
              lineText.content,
              captureIndices
            )
          );
        }
        if (!hasAdvanced && beforePush.hasSameRuleAs(stack)) {
          stack = stack.pop();
          lineTokens.produce(stack, lineLength);
          STOP = true;
          return;
        }
      } else if (_rule instanceof BeginWhileRule) {
        const pushedRule = _rule;
        handleCaptures(
          grammar,
          lineText,
          isFirstLine,
          stack,
          lineTokens,
          pushedRule.beginCaptures,
          captureIndices
        );
        lineTokens.produce(stack, captureIndices[0].end);
        anchorPosition = captureIndices[0].end;
        const contentName = pushedRule.getContentName(
          lineText.content,
          captureIndices
        );
        const contentNameScopesList = nameScopesList.pushAttributed(
          contentName,
          grammar
        );
        stack = stack.withContentNameScopesList(contentNameScopesList);
        if (pushedRule.whileHasBackReferences) {
          stack = stack.withEndRule(
            pushedRule.getWhileWithResolvedBackReferences(
              lineText.content,
              captureIndices
            )
          );
        }
        if (!hasAdvanced && beforePush.hasSameRuleAs(stack)) {
          stack = stack.pop();
          lineTokens.produce(stack, lineLength);
          STOP = true;
          return;
        }
      } else {
        const matchingRule = _rule;
        handleCaptures(
          grammar,
          lineText,
          isFirstLine,
          stack,
          lineTokens,
          matchingRule.captures,
          captureIndices
        );
        lineTokens.produce(stack, captureIndices[0].end);
        stack = stack.pop();
        if (!hasAdvanced) {
          stack = stack.safePop();
          lineTokens.produce(stack, lineLength);
          STOP = true;
          return;
        }
      }
    }
    if (captureIndices[0].end > linePos) {
      linePos = captureIndices[0].end;
      isFirstLine = false;
    }
  }
}
function _checkWhileConditions(grammar, lineText, isFirstLine, linePos, stack, lineTokens) {
  let anchorPosition = stack.beginRuleCapturedEOL ? 0 : -1;
  const whileRules = [];
  for (let node = stack; node; node = node.pop()) {
    const nodeRule = node.getRule(grammar);
    if (nodeRule instanceof BeginWhileRule) {
      whileRules.push({
        rule: nodeRule,
        stack: node
      });
    }
  }
  for (let whileRule = whileRules.pop(); whileRule; whileRule = whileRules.pop()) {
    const { ruleScanner, findOptions } = prepareRuleWhileSearch(whileRule.rule, grammar, whileRule.stack.endRule, isFirstLine, linePos === anchorPosition);
    const r2 = ruleScanner.findNextMatchSync(lineText, linePos, findOptions);
    if (r2) {
      const matchedRuleId = r2.ruleId;
      if (matchedRuleId !== whileRuleId) {
        stack = whileRule.stack.pop();
        break;
      }
      if (r2.captureIndices && r2.captureIndices.length) {
        lineTokens.produce(whileRule.stack, r2.captureIndices[0].start);
        handleCaptures(grammar, lineText, isFirstLine, whileRule.stack, lineTokens, whileRule.rule.whileCaptures, r2.captureIndices);
        lineTokens.produce(whileRule.stack, r2.captureIndices[0].end);
        anchorPosition = r2.captureIndices[0].end;
        if (r2.captureIndices[0].end > linePos) {
          linePos = r2.captureIndices[0].end;
          isFirstLine = false;
        }
      }
    } else {
      stack = whileRule.stack.pop();
      break;
    }
  }
  return { stack, linePos, anchorPosition, isFirstLine };
}
function matchRuleOrInjections(grammar, lineText, isFirstLine, linePos, stack, anchorPosition) {
  const matchResult = matchRule(grammar, lineText, isFirstLine, linePos, stack, anchorPosition);
  const injections = grammar.getInjections();
  if (injections.length === 0) {
    return matchResult;
  }
  const injectionResult = matchInjections(injections, grammar, lineText, isFirstLine, linePos, stack, anchorPosition);
  if (!injectionResult) {
    return matchResult;
  }
  if (!matchResult) {
    return injectionResult;
  }
  const matchResultScore = matchResult.captureIndices[0].start;
  const injectionResultScore = injectionResult.captureIndices[0].start;
  if (injectionResultScore < matchResultScore || injectionResult.priorityMatch && injectionResultScore === matchResultScore) {
    return injectionResult;
  }
  return matchResult;
}
function matchRule(grammar, lineText, isFirstLine, linePos, stack, anchorPosition) {
  const rule = stack.getRule(grammar);
  const { ruleScanner, findOptions } = prepareRuleSearch(rule, grammar, stack.endRule, isFirstLine, linePos === anchorPosition);
  const r2 = ruleScanner.findNextMatchSync(lineText, linePos, findOptions);
  if (r2) {
    return {
      captureIndices: r2.captureIndices,
      matchedRuleId: r2.ruleId
    };
  }
  return null;
}
function matchInjections(injections, grammar, lineText, isFirstLine, linePos, stack, anchorPosition) {
  let bestMatchRating = Number.MAX_VALUE;
  let bestMatchCaptureIndices = null;
  let bestMatchRuleId;
  let bestMatchResultPriority = 0;
  const scopes = stack.contentNameScopesList.getScopeNames();
  for (let i2 = 0, len = injections.length; i2 < len; i2++) {
    const injection = injections[i2];
    if (!injection.matcher(scopes)) {
      continue;
    }
    const rule = grammar.getRule(injection.ruleId);
    const { ruleScanner, findOptions } = prepareRuleSearch(rule, grammar, null, isFirstLine, linePos === anchorPosition);
    const matchResult = ruleScanner.findNextMatchSync(lineText, linePos, findOptions);
    if (!matchResult) {
      continue;
    }
    const matchRating = matchResult.captureIndices[0].start;
    if (matchRating >= bestMatchRating) {
      continue;
    }
    bestMatchRating = matchRating;
    bestMatchCaptureIndices = matchResult.captureIndices;
    bestMatchRuleId = matchResult.ruleId;
    bestMatchResultPriority = injection.priority;
    if (bestMatchRating === linePos) {
      break;
    }
  }
  if (bestMatchCaptureIndices) {
    return {
      priorityMatch: bestMatchResultPriority === -1,
      captureIndices: bestMatchCaptureIndices,
      matchedRuleId: bestMatchRuleId
    };
  }
  return null;
}
function prepareRuleSearch(rule, grammar, endRegexSource, allowA, allowG) {
  const ruleScanner = rule.compileAG(grammar, endRegexSource, allowA, allowG);
  return {
    ruleScanner,
    findOptions: 0
    /* None */
  };
}
function prepareRuleWhileSearch(rule, grammar, endRegexSource, allowA, allowG) {
  const ruleScanner = rule.compileWhileAG(grammar, endRegexSource, allowA, allowG);
  return {
    ruleScanner,
    findOptions: 0
    /* None */
  };
}
function handleCaptures(grammar, lineText, isFirstLine, stack, lineTokens, captures, captureIndices) {
  if (captures.length === 0) {
    return;
  }
  const lineTextContent = lineText.content;
  const len = Math.min(captures.length, captureIndices.length);
  const localStack = [];
  const maxEnd = captureIndices[0].end;
  for (let i2 = 0; i2 < len; i2++) {
    const captureRule = captures[i2];
    if (captureRule === null) {
      continue;
    }
    const captureIndex = captureIndices[i2];
    if (captureIndex.length === 0) {
      continue;
    }
    if (captureIndex.start > maxEnd) {
      break;
    }
    while (localStack.length > 0 && localStack[localStack.length - 1].endPos <= captureIndex.start) {
      lineTokens.produceFromScopes(localStack[localStack.length - 1].scopes, localStack[localStack.length - 1].endPos);
      localStack.pop();
    }
    if (localStack.length > 0) {
      lineTokens.produceFromScopes(localStack[localStack.length - 1].scopes, captureIndex.start);
    } else {
      lineTokens.produce(stack, captureIndex.start);
    }
    if (captureRule.retokenizeCapturedWithRuleId) {
      const scopeName = captureRule.getName(lineTextContent, captureIndices);
      const nameScopesList = stack.contentNameScopesList.pushAttributed(scopeName, grammar);
      const contentName = captureRule.getContentName(lineTextContent, captureIndices);
      const contentNameScopesList = nameScopesList.pushAttributed(contentName, grammar);
      const stackClone = stack.push(captureRule.retokenizeCapturedWithRuleId, captureIndex.start, -1, false, null, nameScopesList, contentNameScopesList);
      const onigSubStr = grammar.createOnigString(lineTextContent.substring(0, captureIndex.end));
      _tokenizeString(
        grammar,
        onigSubStr,
        isFirstLine && captureIndex.start === 0,
        captureIndex.start,
        stackClone,
        lineTokens,
        false,
        /* no time limit */
        0
      );
      disposeOnigString(onigSubStr);
      continue;
    }
    const captureRuleScopeName = captureRule.getName(lineTextContent, captureIndices);
    if (captureRuleScopeName !== null) {
      const base = localStack.length > 0 ? localStack[localStack.length - 1].scopes : stack.contentNameScopesList;
      const captureRuleScopesList = base.pushAttributed(captureRuleScopeName, grammar);
      localStack.push(new LocalStackElement(captureRuleScopesList, captureIndex.end));
    }
  }
  while (localStack.length > 0) {
    lineTokens.produceFromScopes(localStack[localStack.length - 1].scopes, localStack[localStack.length - 1].endPos);
    localStack.pop();
  }
}
var LocalStackElement = class {
  constructor(scopes, endPos) {
    __publicField(this, "scopes");
    __publicField(this, "endPos");
    this.scopes = scopes;
    this.endPos = endPos;
  }
};
function createGrammar(scopeName, grammar, initialLanguage, embeddedLanguages, tokenTypes, balancedBracketSelectors, grammarRepository, onigLib) {
  return new Grammar(
    scopeName,
    grammar,
    initialLanguage,
    embeddedLanguages,
    tokenTypes,
    balancedBracketSelectors,
    grammarRepository,
    onigLib
  );
}
function collectInjections(result, selector, rule, ruleFactoryHelper, grammar) {
  const matchers = createMatchers(selector, nameMatcher);
  const ruleId = RuleFactory.getCompiledRuleId(rule, ruleFactoryHelper, grammar.repository);
  for (const matcher of matchers) {
    result.push({
      debugSelector: selector,
      matcher: matcher.matcher,
      ruleId,
      grammar,
      priority: matcher.priority
    });
  }
}
function nameMatcher(identifers, scopes) {
  if (scopes.length < identifers.length) {
    return false;
  }
  let lastIndex = 0;
  return identifers.every((identifier) => {
    for (let i2 = lastIndex; i2 < scopes.length; i2++) {
      if (scopesAreMatching(scopes[i2], identifier)) {
        lastIndex = i2 + 1;
        return true;
      }
    }
    return false;
  });
}
function scopesAreMatching(thisScopeName, scopeName) {
  if (!thisScopeName) {
    return false;
  }
  if (thisScopeName === scopeName) {
    return true;
  }
  const len = scopeName.length;
  return thisScopeName.length > len && thisScopeName.substr(0, len) === scopeName && thisScopeName[len] === ".";
}
var Grammar = class {
  constructor(_rootScopeName, grammar, initialLanguage, embeddedLanguages, tokenTypes, balancedBracketSelectors, grammarRepository, _onigLib) {
    __publicField(this, "_rootId");
    __publicField(this, "_lastRuleId");
    __publicField(this, "_ruleId2desc");
    __publicField(this, "_includedGrammars");
    __publicField(this, "_grammarRepository");
    __publicField(this, "_grammar");
    __publicField(this, "_injections");
    __publicField(this, "_basicScopeAttributesProvider");
    __publicField(this, "_tokenTypeMatchers");
    this._rootScopeName = _rootScopeName;
    this.balancedBracketSelectors = balancedBracketSelectors;
    this._onigLib = _onigLib;
    this._basicScopeAttributesProvider = new BasicScopeAttributesProvider(
      initialLanguage,
      embeddedLanguages
    );
    this._rootId = -1;
    this._lastRuleId = 0;
    this._ruleId2desc = [null];
    this._includedGrammars = {};
    this._grammarRepository = grammarRepository;
    this._grammar = initGrammar(grammar, null);
    this._injections = null;
    this._tokenTypeMatchers = [];
    if (tokenTypes) {
      for (const selector of Object.keys(tokenTypes)) {
        const matchers = createMatchers(selector, nameMatcher);
        for (const matcher of matchers) {
          this._tokenTypeMatchers.push({
            matcher: matcher.matcher,
            type: tokenTypes[selector]
          });
        }
      }
    }
  }
  get themeProvider() {
    return this._grammarRepository;
  }
  dispose() {
    for (const rule of this._ruleId2desc) {
      if (rule) {
        rule.dispose();
      }
    }
  }
  createOnigScanner(sources) {
    return this._onigLib.createOnigScanner(sources);
  }
  createOnigString(sources) {
    return this._onigLib.createOnigString(sources);
  }
  getMetadataForScope(scope) {
    return this._basicScopeAttributesProvider.getBasicScopeAttributes(scope);
  }
  _collectInjections() {
    const grammarRepository = {
      lookup: (scopeName2) => {
        if (scopeName2 === this._rootScopeName) {
          return this._grammar;
        }
        return this.getExternalGrammar(scopeName2);
      },
      injections: (scopeName2) => {
        return this._grammarRepository.injections(scopeName2);
      }
    };
    const result = [];
    const scopeName = this._rootScopeName;
    const grammar = grammarRepository.lookup(scopeName);
    if (grammar) {
      const rawInjections = grammar.injections;
      if (rawInjections) {
        for (let expression in rawInjections) {
          collectInjections(
            result,
            expression,
            rawInjections[expression],
            this,
            grammar
          );
        }
      }
      const injectionScopeNames = this._grammarRepository.injections(scopeName);
      if (injectionScopeNames) {
        injectionScopeNames.forEach((injectionScopeName) => {
          const injectionGrammar = this.getExternalGrammar(injectionScopeName);
          if (injectionGrammar) {
            const selector = injectionGrammar.injectionSelector;
            if (selector) {
              collectInjections(
                result,
                selector,
                injectionGrammar,
                this,
                injectionGrammar
              );
            }
          }
        });
      }
    }
    result.sort((i1, i2) => i1.priority - i2.priority);
    return result;
  }
  getInjections() {
    if (this._injections === null) {
      this._injections = this._collectInjections();
    }
    return this._injections;
  }
  registerRule(factory) {
    const id = ++this._lastRuleId;
    const result = factory(ruleIdFromNumber(id));
    this._ruleId2desc[id] = result;
    return result;
  }
  getRule(ruleId) {
    return this._ruleId2desc[ruleIdToNumber(ruleId)];
  }
  getExternalGrammar(scopeName, repository) {
    if (this._includedGrammars[scopeName]) {
      return this._includedGrammars[scopeName];
    } else if (this._grammarRepository) {
      const rawIncludedGrammar = this._grammarRepository.lookup(scopeName);
      if (rawIncludedGrammar) {
        this._includedGrammars[scopeName] = initGrammar(
          rawIncludedGrammar,
          repository && repository.$base
        );
        return this._includedGrammars[scopeName];
      }
    }
    return void 0;
  }
  tokenizeLine(lineText, prevState, timeLimit = 0) {
    const r2 = this._tokenize(lineText, prevState, false, timeLimit);
    return {
      tokens: r2.lineTokens.getResult(r2.ruleStack, r2.lineLength),
      ruleStack: r2.ruleStack,
      stoppedEarly: r2.stoppedEarly
    };
  }
  tokenizeLine2(lineText, prevState, timeLimit = 0) {
    const r2 = this._tokenize(lineText, prevState, true, timeLimit);
    return {
      tokens: r2.lineTokens.getBinaryResult(r2.ruleStack, r2.lineLength),
      ruleStack: r2.ruleStack,
      stoppedEarly: r2.stoppedEarly
    };
  }
  _tokenize(lineText, prevState, emitBinaryTokens, timeLimit) {
    if (this._rootId === -1) {
      this._rootId = RuleFactory.getCompiledRuleId(
        this._grammar.repository.$self,
        this,
        this._grammar.repository
      );
      this.getInjections();
    }
    let isFirstLine;
    if (!prevState || prevState === StateStackImpl.NULL) {
      isFirstLine = true;
      const rawDefaultMetadata = this._basicScopeAttributesProvider.getDefaultAttributes();
      const defaultStyle = this.themeProvider.getDefaults();
      const defaultMetadata = EncodedTokenMetadata.set(
        0,
        rawDefaultMetadata.languageId,
        rawDefaultMetadata.tokenType,
        null,
        defaultStyle.fontStyle,
        defaultStyle.foregroundId,
        defaultStyle.backgroundId
      );
      const rootScopeName = this.getRule(this._rootId).getName(
        null,
        null
      );
      let scopeList;
      if (rootScopeName) {
        scopeList = AttributedScopeStack.createRootAndLookUpScopeName(
          rootScopeName,
          defaultMetadata,
          this
        );
      } else {
        scopeList = AttributedScopeStack.createRoot(
          "unknown",
          defaultMetadata
        );
      }
      prevState = new StateStackImpl(
        null,
        this._rootId,
        -1,
        -1,
        false,
        null,
        scopeList,
        scopeList
      );
    } else {
      isFirstLine = false;
      prevState.reset();
    }
    lineText = lineText + "\n";
    const onigLineText = this.createOnigString(lineText);
    const lineLength = onigLineText.content.length;
    const lineTokens = new LineTokens(
      emitBinaryTokens,
      lineText,
      this._tokenTypeMatchers,
      this.balancedBracketSelectors
    );
    const r2 = _tokenizeString(
      this,
      onigLineText,
      isFirstLine,
      0,
      prevState,
      lineTokens,
      true,
      timeLimit
    );
    disposeOnigString(onigLineText);
    return {
      lineLength,
      lineTokens,
      ruleStack: r2.stack,
      stoppedEarly: r2.stoppedEarly
    };
  }
};
function initGrammar(grammar, base) {
  grammar = clone(grammar);
  grammar.repository = grammar.repository || {};
  grammar.repository.$self = {
    $vscodeTextmateLocation: grammar.$vscodeTextmateLocation,
    patterns: grammar.patterns,
    name: grammar.scopeName
  };
  grammar.repository.$base = base || grammar.repository.$self;
  return grammar;
}
var AttributedScopeStack = class _AttributedScopeStack {
  /**
   * Invariant:
   * ```
   * if (parent && !scopePath.extends(parent.scopePath)) {
   * 	throw new Error();
   * }
   * ```
   */
  constructor(parent, scopePath, tokenAttributes) {
    this.parent = parent;
    this.scopePath = scopePath;
    this.tokenAttributes = tokenAttributes;
  }
  static fromExtension(namesScopeList, contentNameScopesList) {
    let current = namesScopeList;
    let scopeNames = (namesScopeList == null ? void 0 : namesScopeList.scopePath) ?? null;
    for (const frame of contentNameScopesList) {
      scopeNames = ScopeStack.push(scopeNames, frame.scopeNames);
      current = new _AttributedScopeStack(current, scopeNames, frame.encodedTokenAttributes);
    }
    return current;
  }
  static createRoot(scopeName, tokenAttributes) {
    return new _AttributedScopeStack(null, new ScopeStack(null, scopeName), tokenAttributes);
  }
  static createRootAndLookUpScopeName(scopeName, tokenAttributes, grammar) {
    const rawRootMetadata = grammar.getMetadataForScope(scopeName);
    const scopePath = new ScopeStack(null, scopeName);
    const rootStyle = grammar.themeProvider.themeMatch(scopePath);
    const resolvedTokenAttributes = _AttributedScopeStack.mergeAttributes(
      tokenAttributes,
      rawRootMetadata,
      rootStyle
    );
    return new _AttributedScopeStack(null, scopePath, resolvedTokenAttributes);
  }
  get scopeName() {
    return this.scopePath.scopeName;
  }
  toString() {
    return this.getScopeNames().join(" ");
  }
  equals(other) {
    return _AttributedScopeStack.equals(this, other);
  }
  static equals(a, b2) {
    do {
      if (a === b2) {
        return true;
      }
      if (!a && !b2) {
        return true;
      }
      if (!a || !b2) {
        return false;
      }
      if (a.scopeName !== b2.scopeName || a.tokenAttributes !== b2.tokenAttributes) {
        return false;
      }
      a = a.parent;
      b2 = b2.parent;
    } while (true);
  }
  static mergeAttributes(existingTokenAttributes, basicScopeAttributes, styleAttributes) {
    let fontStyle = -1;
    let foreground = 0;
    let background = 0;
    if (styleAttributes !== null) {
      fontStyle = styleAttributes.fontStyle;
      foreground = styleAttributes.foregroundId;
      background = styleAttributes.backgroundId;
    }
    return EncodedTokenMetadata.set(
      existingTokenAttributes,
      basicScopeAttributes.languageId,
      basicScopeAttributes.tokenType,
      null,
      fontStyle,
      foreground,
      background
    );
  }
  pushAttributed(scopePath, grammar) {
    if (scopePath === null) {
      return this;
    }
    if (scopePath.indexOf(" ") === -1) {
      return _AttributedScopeStack._pushAttributed(this, scopePath, grammar);
    }
    const scopes = scopePath.split(/ /g);
    let result = this;
    for (const scope of scopes) {
      result = _AttributedScopeStack._pushAttributed(result, scope, grammar);
    }
    return result;
  }
  static _pushAttributed(target, scopeName, grammar) {
    const rawMetadata = grammar.getMetadataForScope(scopeName);
    const newPath = target.scopePath.push(scopeName);
    const scopeThemeMatchResult = grammar.themeProvider.themeMatch(newPath);
    const metadata = _AttributedScopeStack.mergeAttributes(
      target.tokenAttributes,
      rawMetadata,
      scopeThemeMatchResult
    );
    return new _AttributedScopeStack(target, newPath, metadata);
  }
  getScopeNames() {
    return this.scopePath.getSegments();
  }
  getExtensionIfDefined(base) {
    var _a2;
    const result = [];
    let self = this;
    while (self && self !== base) {
      result.push({
        encodedTokenAttributes: self.tokenAttributes,
        scopeNames: self.scopePath.getExtensionIfDefined(((_a2 = self.parent) == null ? void 0 : _a2.scopePath) ?? null)
      });
      self = self.parent;
    }
    return self === base ? result.reverse() : void 0;
  }
};
var StateStackImpl = (_b = class {
  /**
   * Invariant:
   * ```
   * if (contentNameScopesList !== nameScopesList && contentNameScopesList?.parent !== nameScopesList) {
   * 	throw new Error();
   * }
   * if (this.parent && !nameScopesList.extends(this.parent.contentNameScopesList)) {
   * 	throw new Error();
   * }
   * ```
   */
  constructor(parent, ruleId, enterPos, anchorPos, beginRuleCapturedEOL, endRule, nameScopesList, contentNameScopesList) {
    __publicField(this, "_stackElementBrand");
    /**
     * The position on the current line where this state was pushed.
     * This is relevant only while tokenizing a line, to detect endless loops.
     * Its value is meaningless across lines.
     */
    __publicField(this, "_enterPos");
    /**
     * The captured anchor position when this stack element was pushed.
     * This is relevant only while tokenizing a line, to restore the anchor position when popping.
     * Its value is meaningless across lines.
     */
    __publicField(this, "_anchorPos");
    /**
     * The depth of the stack.
     */
    __publicField(this, "depth");
    this.parent = parent;
    this.ruleId = ruleId;
    this.beginRuleCapturedEOL = beginRuleCapturedEOL;
    this.endRule = endRule;
    this.nameScopesList = nameScopesList;
    this.contentNameScopesList = contentNameScopesList;
    this.depth = this.parent ? this.parent.depth + 1 : 1;
    this._enterPos = enterPos;
    this._anchorPos = anchorPos;
  }
  equals(other) {
    if (other === null) {
      return false;
    }
    return _b._equals(this, other);
  }
  static _equals(a, b2) {
    if (a === b2) {
      return true;
    }
    if (!this._structuralEquals(a, b2)) {
      return false;
    }
    return AttributedScopeStack.equals(a.contentNameScopesList, b2.contentNameScopesList);
  }
  /**
   * A structural equals check. Does not take into account `scopes`.
   */
  static _structuralEquals(a, b2) {
    do {
      if (a === b2) {
        return true;
      }
      if (!a && !b2) {
        return true;
      }
      if (!a || !b2) {
        return false;
      }
      if (a.depth !== b2.depth || a.ruleId !== b2.ruleId || a.endRule !== b2.endRule) {
        return false;
      }
      a = a.parent;
      b2 = b2.parent;
    } while (true);
  }
  clone() {
    return this;
  }
  static _reset(el) {
    while (el) {
      el._enterPos = -1;
      el._anchorPos = -1;
      el = el.parent;
    }
  }
  reset() {
    _b._reset(this);
  }
  pop() {
    return this.parent;
  }
  safePop() {
    if (this.parent) {
      return this.parent;
    }
    return this;
  }
  push(ruleId, enterPos, anchorPos, beginRuleCapturedEOL, endRule, nameScopesList, contentNameScopesList) {
    return new _b(
      this,
      ruleId,
      enterPos,
      anchorPos,
      beginRuleCapturedEOL,
      endRule,
      nameScopesList,
      contentNameScopesList
    );
  }
  getEnterPos() {
    return this._enterPos;
  }
  getAnchorPos() {
    return this._anchorPos;
  }
  getRule(grammar) {
    return grammar.getRule(this.ruleId);
  }
  toString() {
    const r2 = [];
    this._writeString(r2, 0);
    return "[" + r2.join(",") + "]";
  }
  _writeString(res, outIndex) {
    var _a2, _b2;
    if (this.parent) {
      outIndex = this.parent._writeString(res, outIndex);
    }
    res[outIndex++] = `(${this.ruleId}, ${(_a2 = this.nameScopesList) == null ? void 0 : _a2.toString()}, ${(_b2 = this.contentNameScopesList) == null ? void 0 : _b2.toString()})`;
    return outIndex;
  }
  withContentNameScopesList(contentNameScopeStack) {
    if (this.contentNameScopesList === contentNameScopeStack) {
      return this;
    }
    return this.parent.push(
      this.ruleId,
      this._enterPos,
      this._anchorPos,
      this.beginRuleCapturedEOL,
      this.endRule,
      this.nameScopesList,
      contentNameScopeStack
    );
  }
  withEndRule(endRule) {
    if (this.endRule === endRule) {
      return this;
    }
    return new _b(
      this.parent,
      this.ruleId,
      this._enterPos,
      this._anchorPos,
      this.beginRuleCapturedEOL,
      endRule,
      this.nameScopesList,
      this.contentNameScopesList
    );
  }
  // Used to warn of endless loops
  hasSameRuleAs(other) {
    let el = this;
    while (el && el._enterPos === other._enterPos) {
      if (el.ruleId === other.ruleId) {
        return true;
      }
      el = el.parent;
    }
    return false;
  }
  toStateStackFrame() {
    var _a2, _b2, _c2;
    return {
      ruleId: ruleIdToNumber(this.ruleId),
      beginRuleCapturedEOL: this.beginRuleCapturedEOL,
      endRule: this.endRule,
      nameScopesList: ((_b2 = this.nameScopesList) == null ? void 0 : _b2.getExtensionIfDefined(((_a2 = this.parent) == null ? void 0 : _a2.nameScopesList) ?? null)) ?? [],
      contentNameScopesList: ((_c2 = this.contentNameScopesList) == null ? void 0 : _c2.getExtensionIfDefined(this.nameScopesList)) ?? []
    };
  }
  static pushFrame(self, frame) {
    const namesScopeList = AttributedScopeStack.fromExtension((self == null ? void 0 : self.nameScopesList) ?? null, frame.nameScopesList);
    return new _b(
      self,
      ruleIdFromNumber(frame.ruleId),
      frame.enterPos ?? -1,
      frame.anchorPos ?? -1,
      frame.beginRuleCapturedEOL,
      frame.endRule,
      namesScopeList,
      AttributedScopeStack.fromExtension(namesScopeList, frame.contentNameScopesList)
    );
  }
}, // TODO remove me
__publicField(_b, "NULL", new _b(
  null,
  0,
  0,
  0,
  false,
  null,
  null,
  null
)), _b);
var BalancedBracketSelectors = class {
  constructor(balancedBracketScopes, unbalancedBracketScopes) {
    __publicField(this, "balancedBracketScopes");
    __publicField(this, "unbalancedBracketScopes");
    __publicField(this, "allowAny", false);
    this.balancedBracketScopes = balancedBracketScopes.flatMap(
      (selector) => {
        if (selector === "*") {
          this.allowAny = true;
          return [];
        }
        return createMatchers(selector, nameMatcher).map((m2) => m2.matcher);
      }
    );
    this.unbalancedBracketScopes = unbalancedBracketScopes.flatMap(
      (selector) => createMatchers(selector, nameMatcher).map((m2) => m2.matcher)
    );
  }
  get matchesAlways() {
    return this.allowAny && this.unbalancedBracketScopes.length === 0;
  }
  get matchesNever() {
    return this.balancedBracketScopes.length === 0 && !this.allowAny;
  }
  match(scopes) {
    for (const excluder of this.unbalancedBracketScopes) {
      if (excluder(scopes)) {
        return false;
      }
    }
    for (const includer of this.balancedBracketScopes) {
      if (includer(scopes)) {
        return true;
      }
    }
    return this.allowAny;
  }
};
var LineTokens = class {
  constructor(emitBinaryTokens, lineText, tokenTypeOverrides, balancedBracketSelectors) {
    __publicField(this, "_emitBinaryTokens");
    /**
     * defined only if `false`.
     */
    __publicField(this, "_lineText");
    /**
     * used only if `_emitBinaryTokens` is false.
     */
    __publicField(this, "_tokens");
    /**
     * used only if `_emitBinaryTokens` is true.
     */
    __publicField(this, "_binaryTokens");
    __publicField(this, "_lastTokenEndIndex");
    __publicField(this, "_tokenTypeOverrides");
    this.balancedBracketSelectors = balancedBracketSelectors;
    this._emitBinaryTokens = emitBinaryTokens;
    this._tokenTypeOverrides = tokenTypeOverrides;
    {
      this._lineText = null;
    }
    this._tokens = [];
    this._binaryTokens = [];
    this._lastTokenEndIndex = 0;
  }
  produce(stack, endIndex) {
    this.produceFromScopes(stack.contentNameScopesList, endIndex);
  }
  produceFromScopes(scopesList, endIndex) {
    var _a2;
    if (this._lastTokenEndIndex >= endIndex) {
      return;
    }
    if (this._emitBinaryTokens) {
      let metadata = (scopesList == null ? void 0 : scopesList.tokenAttributes) ?? 0;
      let containsBalancedBrackets = false;
      if ((_a2 = this.balancedBracketSelectors) == null ? void 0 : _a2.matchesAlways) {
        containsBalancedBrackets = true;
      }
      if (this._tokenTypeOverrides.length > 0 || this.balancedBracketSelectors && !this.balancedBracketSelectors.matchesAlways && !this.balancedBracketSelectors.matchesNever) {
        const scopes2 = (scopesList == null ? void 0 : scopesList.getScopeNames()) ?? [];
        for (const tokenType of this._tokenTypeOverrides) {
          if (tokenType.matcher(scopes2)) {
            metadata = EncodedTokenMetadata.set(
              metadata,
              0,
              toOptionalTokenType(tokenType.type),
              null,
              -1,
              0,
              0
            );
          }
        }
        if (this.balancedBracketSelectors) {
          containsBalancedBrackets = this.balancedBracketSelectors.match(scopes2);
        }
      }
      if (containsBalancedBrackets) {
        metadata = EncodedTokenMetadata.set(
          metadata,
          0,
          8,
          containsBalancedBrackets,
          -1,
          0,
          0
        );
      }
      if (this._binaryTokens.length > 0 && this._binaryTokens[this._binaryTokens.length - 1] === metadata) {
        this._lastTokenEndIndex = endIndex;
        return;
      }
      this._binaryTokens.push(this._lastTokenEndIndex);
      this._binaryTokens.push(metadata);
      this._lastTokenEndIndex = endIndex;
      return;
    }
    const scopes = (scopesList == null ? void 0 : scopesList.getScopeNames()) ?? [];
    this._tokens.push({
      startIndex: this._lastTokenEndIndex,
      endIndex,
      // value: lineText.substring(lastTokenEndIndex, endIndex),
      scopes
    });
    this._lastTokenEndIndex = endIndex;
  }
  getResult(stack, lineLength) {
    if (this._tokens.length > 0 && this._tokens[this._tokens.length - 1].startIndex === lineLength - 1) {
      this._tokens.pop();
    }
    if (this._tokens.length === 0) {
      this._lastTokenEndIndex = -1;
      this.produce(stack, lineLength);
      this._tokens[this._tokens.length - 1].startIndex = 0;
    }
    return this._tokens;
  }
  getBinaryResult(stack, lineLength) {
    if (this._binaryTokens.length > 0 && this._binaryTokens[this._binaryTokens.length - 2] === lineLength - 1) {
      this._binaryTokens.pop();
      this._binaryTokens.pop();
    }
    if (this._binaryTokens.length === 0) {
      this._lastTokenEndIndex = -1;
      this.produce(stack, lineLength);
      this._binaryTokens[this._binaryTokens.length - 2] = 0;
    }
    const result = new Uint32Array(this._binaryTokens.length);
    for (let i2 = 0, len = this._binaryTokens.length; i2 < len; i2++) {
      result[i2] = this._binaryTokens[i2];
    }
    return result;
  }
};
var SyncRegistry = class {
  constructor(theme, _onigLib) {
    __publicField(this, "_grammars", /* @__PURE__ */ new Map());
    __publicField(this, "_rawGrammars", /* @__PURE__ */ new Map());
    __publicField(this, "_injectionGrammars", /* @__PURE__ */ new Map());
    __publicField(this, "_theme");
    this._onigLib = _onigLib;
    this._theme = theme;
  }
  dispose() {
    for (const grammar of this._grammars.values()) {
      grammar.dispose();
    }
  }
  setTheme(theme) {
    this._theme = theme;
  }
  getColorMap() {
    return this._theme.getColorMap();
  }
  /**
   * Add `grammar` to registry and return a list of referenced scope names
   */
  addGrammar(grammar, injectionScopeNames) {
    this._rawGrammars.set(grammar.scopeName, grammar);
    if (injectionScopeNames) {
      this._injectionGrammars.set(grammar.scopeName, injectionScopeNames);
    }
  }
  /**
   * Lookup a raw grammar.
   */
  lookup(scopeName) {
    return this._rawGrammars.get(scopeName);
  }
  /**
   * Returns the injections for the given grammar
   */
  injections(targetScope) {
    return this._injectionGrammars.get(targetScope);
  }
  /**
   * Get the default theme settings
   */
  getDefaults() {
    return this._theme.getDefaults();
  }
  /**
   * Match a scope in the theme.
   */
  themeMatch(scopePath) {
    return this._theme.match(scopePath);
  }
  /**
   * Lookup a grammar.
   */
  grammarForScopeName(scopeName, initialLanguage, embeddedLanguages, tokenTypes, balancedBracketSelectors) {
    if (!this._grammars.has(scopeName)) {
      let rawGrammar = this._rawGrammars.get(scopeName);
      if (!rawGrammar) {
        return null;
      }
      this._grammars.set(scopeName, createGrammar(
        scopeName,
        rawGrammar,
        initialLanguage,
        embeddedLanguages,
        tokenTypes,
        balancedBracketSelectors,
        this,
        this._onigLib
      ));
    }
    return this._grammars.get(scopeName);
  }
};
var Registry$1 = class Registry {
  constructor(options) {
    __publicField(this, "_options");
    __publicField(this, "_syncRegistry");
    __publicField(this, "_ensureGrammarCache");
    this._options = options;
    this._syncRegistry = new SyncRegistry(
      Theme.createFromRawTheme(options.theme, options.colorMap),
      options.onigLib
    );
    this._ensureGrammarCache = /* @__PURE__ */ new Map();
  }
  dispose() {
    this._syncRegistry.dispose();
  }
  /**
   * Change the theme. Once called, no previous `ruleStack` should be used anymore.
   */
  setTheme(theme, colorMap) {
    this._syncRegistry.setTheme(Theme.createFromRawTheme(theme, colorMap));
  }
  /**
   * Returns a lookup array for color ids.
   */
  getColorMap() {
    return this._syncRegistry.getColorMap();
  }
  /**
   * Load the grammar for `scopeName` and all referenced included grammars asynchronously.
   * Please do not use language id 0.
   */
  loadGrammarWithEmbeddedLanguages(initialScopeName, initialLanguage, embeddedLanguages) {
    return this.loadGrammarWithConfiguration(initialScopeName, initialLanguage, { embeddedLanguages });
  }
  /**
   * Load the grammar for `scopeName` and all referenced included grammars asynchronously.
   * Please do not use language id 0.
   */
  loadGrammarWithConfiguration(initialScopeName, initialLanguage, configuration) {
    return this._loadGrammar(
      initialScopeName,
      initialLanguage,
      configuration.embeddedLanguages,
      configuration.tokenTypes,
      new BalancedBracketSelectors(
        configuration.balancedBracketSelectors || [],
        configuration.unbalancedBracketSelectors || []
      )
    );
  }
  /**
   * Load the grammar for `scopeName` and all referenced included grammars asynchronously.
   */
  loadGrammar(initialScopeName) {
    return this._loadGrammar(initialScopeName, 0, null, null, null);
  }
  _loadGrammar(initialScopeName, initialLanguage, embeddedLanguages, tokenTypes, balancedBracketSelectors) {
    const dependencyProcessor = new ScopeDependencyProcessor(this._syncRegistry, initialScopeName);
    while (dependencyProcessor.Q.length > 0) {
      dependencyProcessor.Q.map((request) => this._loadSingleGrammar(request.scopeName));
      dependencyProcessor.processQueue();
    }
    return this._grammarForScopeName(
      initialScopeName,
      initialLanguage,
      embeddedLanguages,
      tokenTypes,
      balancedBracketSelectors
    );
  }
  _loadSingleGrammar(scopeName) {
    if (!this._ensureGrammarCache.has(scopeName)) {
      this._doLoadSingleGrammar(scopeName);
      this._ensureGrammarCache.set(scopeName, true);
    }
  }
  _doLoadSingleGrammar(scopeName) {
    const grammar = this._options.loadGrammar(scopeName);
    if (grammar) {
      const injections = typeof this._options.getInjections === "function" ? this._options.getInjections(scopeName) : void 0;
      this._syncRegistry.addGrammar(grammar, injections);
    }
  }
  /**
   * Adds a rawGrammar.
   */
  addGrammar(rawGrammar, injections = [], initialLanguage = 0, embeddedLanguages = null) {
    this._syncRegistry.addGrammar(rawGrammar, injections);
    return this._grammarForScopeName(rawGrammar.scopeName, initialLanguage, embeddedLanguages);
  }
  /**
   * Get the grammar for `scopeName`. The grammar must first be created via `loadGrammar` or `addGrammar`.
   */
  _grammarForScopeName(scopeName, initialLanguage = 0, embeddedLanguages = null, tokenTypes = null, balancedBracketSelectors = null) {
    return this._syncRegistry.grammarForScopeName(
      scopeName,
      initialLanguage,
      embeddedLanguages,
      tokenTypes,
      balancedBracketSelectors
    );
  }
};
var INITIAL = StateStackImpl.NULL;
const htmlVoidElements = [
  "area",
  "base",
  "basefont",
  "bgsound",
  "br",
  "col",
  "command",
  "embed",
  "frame",
  "hr",
  "image",
  "img",
  "input",
  "keygen",
  "link",
  "meta",
  "param",
  "source",
  "track",
  "wbr"
];
class Schema {
  /**
   * @param {SchemaType['property']} property
   *   Property.
   * @param {SchemaType['normal']} normal
   *   Normal.
   * @param {Space | undefined} [space]
   *   Space.
   * @returns
   *   Schema.
   */
  constructor(property, normal, space) {
    this.normal = normal;
    this.property = property;
    if (space) {
      this.space = space;
    }
  }
}
Schema.prototype.normal = {};
Schema.prototype.property = {};
Schema.prototype.space = void 0;
function merge(definitions, space) {
  const property = {};
  const normal = {};
  for (const definition of definitions) {
    Object.assign(property, definition.property);
    Object.assign(normal, definition.normal);
  }
  return new Schema(property, normal, space);
}
function normalize(value) {
  return value.toLowerCase();
}
class Info {
  /**
   * @param {string} property
   *   Property.
   * @param {string} attribute
   *   Attribute.
   * @returns
   *   Info.
   */
  constructor(property, attribute) {
    this.attribute = attribute;
    this.property = property;
  }
}
Info.prototype.attribute = "";
Info.prototype.booleanish = false;
Info.prototype.boolean = false;
Info.prototype.commaOrSpaceSeparated = false;
Info.prototype.commaSeparated = false;
Info.prototype.defined = false;
Info.prototype.mustUseProperty = false;
Info.prototype.number = false;
Info.prototype.overloadedBoolean = false;
Info.prototype.property = "";
Info.prototype.spaceSeparated = false;
Info.prototype.space = void 0;
let powers = 0;
const boolean = increment();
const booleanish = increment();
const overloadedBoolean = increment();
const number = increment();
const spaceSeparated = increment();
const commaSeparated = increment();
const commaOrSpaceSeparated = increment();
function increment() {
  return 2 ** ++powers;
}
const types = /* @__PURE__ */ Object.freeze(/* @__PURE__ */ Object.defineProperty({
  __proto__: null,
  boolean,
  booleanish,
  commaOrSpaceSeparated,
  commaSeparated,
  number,
  overloadedBoolean,
  spaceSeparated
}, Symbol.toStringTag, { value: "Module" }));
const checks = (
  /** @type {ReadonlyArray<keyof typeof types>} */
  Object.keys(types)
);
class DefinedInfo extends Info {
  /**
   * @constructor
   * @param {string} property
   *   Property.
   * @param {string} attribute
   *   Attribute.
   * @param {number | null | undefined} [mask]
   *   Mask.
   * @param {Space | undefined} [space]
   *   Space.
   * @returns
   *   Info.
   */
  constructor(property, attribute, mask, space) {
    let index = -1;
    super(property, attribute);
    mark(this, "space", space);
    if (typeof mask === "number") {
      while (++index < checks.length) {
        const check = checks[index];
        mark(this, checks[index], (mask & types[check]) === types[check]);
      }
    }
  }
}
DefinedInfo.prototype.defined = true;
function mark(values, key2, value) {
  if (value) {
    values[key2] = value;
  }
}
function create(definition) {
  const properties = {};
  const normals = {};
  for (const [property, value] of Object.entries(definition.properties)) {
    const info = new DefinedInfo(
      property,
      definition.transform(definition.attributes || {}, property),
      value,
      definition.space
    );
    if (definition.mustUseProperty && definition.mustUseProperty.includes(property)) {
      info.mustUseProperty = true;
    }
    properties[property] = info;
    normals[normalize(property)] = property;
    normals[normalize(info.attribute)] = property;
  }
  return new Schema(properties, normals, definition.space);
}
const aria = create({
  properties: {
    ariaActiveDescendant: null,
    ariaAtomic: booleanish,
    ariaAutoComplete: null,
    ariaBusy: booleanish,
    ariaChecked: booleanish,
    ariaColCount: number,
    ariaColIndex: number,
    ariaColSpan: number,
    ariaControls: spaceSeparated,
    ariaCurrent: null,
    ariaDescribedBy: spaceSeparated,
    ariaDetails: null,
    ariaDisabled: booleanish,
    ariaDropEffect: spaceSeparated,
    ariaErrorMessage: null,
    ariaExpanded: booleanish,
    ariaFlowTo: spaceSeparated,
    ariaGrabbed: booleanish,
    ariaHasPopup: null,
    ariaHidden: booleanish,
    ariaInvalid: null,
    ariaKeyShortcuts: null,
    ariaLabel: null,
    ariaLabelledBy: spaceSeparated,
    ariaLevel: number,
    ariaLive: null,
    ariaModal: booleanish,
    ariaMultiLine: booleanish,
    ariaMultiSelectable: booleanish,
    ariaOrientation: null,
    ariaOwns: spaceSeparated,
    ariaPlaceholder: null,
    ariaPosInSet: number,
    ariaPressed: booleanish,
    ariaReadOnly: booleanish,
    ariaRelevant: null,
    ariaRequired: booleanish,
    ariaRoleDescription: spaceSeparated,
    ariaRowCount: number,
    ariaRowIndex: number,
    ariaRowSpan: number,
    ariaSelected: booleanish,
    ariaSetSize: number,
    ariaSort: null,
    ariaValueMax: number,
    ariaValueMin: number,
    ariaValueNow: number,
    ariaValueText: null,
    role: null
  },
  transform(_2, property) {
    return property === "role" ? property : "aria-" + property.slice(4).toLowerCase();
  }
});
function caseSensitiveTransform(attributes, attribute) {
  return attribute in attributes ? attributes[attribute] : attribute;
}
function caseInsensitiveTransform(attributes, property) {
  return caseSensitiveTransform(attributes, property.toLowerCase());
}
const html$3 = create({
  attributes: {
    acceptcharset: "accept-charset",
    classname: "class",
    htmlfor: "for",
    httpequiv: "http-equiv"
  },
  mustUseProperty: ["checked", "multiple", "muted", "selected"],
  properties: {
    // Standard Properties.
    abbr: null,
    accept: commaSeparated,
    acceptCharset: spaceSeparated,
    accessKey: spaceSeparated,
    action: null,
    allow: null,
    allowFullScreen: boolean,
    allowPaymentRequest: boolean,
    allowUserMedia: boolean,
    alt: null,
    as: null,
    async: boolean,
    autoCapitalize: null,
    autoComplete: spaceSeparated,
    autoFocus: boolean,
    autoPlay: boolean,
    blocking: spaceSeparated,
    capture: null,
    charSet: null,
    checked: boolean,
    cite: null,
    className: spaceSeparated,
    cols: number,
    colSpan: null,
    content: null,
    contentEditable: booleanish,
    controls: boolean,
    controlsList: spaceSeparated,
    coords: number | commaSeparated,
    crossOrigin: null,
    data: null,
    dateTime: null,
    decoding: null,
    default: boolean,
    defer: boolean,
    dir: null,
    dirName: null,
    disabled: boolean,
    download: overloadedBoolean,
    draggable: booleanish,
    encType: null,
    enterKeyHint: null,
    fetchPriority: null,
    form: null,
    formAction: null,
    formEncType: null,
    formMethod: null,
    formNoValidate: boolean,
    formTarget: null,
    headers: spaceSeparated,
    height: number,
    hidden: overloadedBoolean,
    high: number,
    href: null,
    hrefLang: null,
    htmlFor: spaceSeparated,
    httpEquiv: spaceSeparated,
    id: null,
    imageSizes: null,
    imageSrcSet: null,
    inert: boolean,
    inputMode: null,
    integrity: null,
    is: null,
    isMap: boolean,
    itemId: null,
    itemProp: spaceSeparated,
    itemRef: spaceSeparated,
    itemScope: boolean,
    itemType: spaceSeparated,
    kind: null,
    label: null,
    lang: null,
    language: null,
    list: null,
    loading: null,
    loop: boolean,
    low: number,
    manifest: null,
    max: null,
    maxLength: number,
    media: null,
    method: null,
    min: null,
    minLength: number,
    multiple: boolean,
    muted: boolean,
    name: null,
    nonce: null,
    noModule: boolean,
    noValidate: boolean,
    onAbort: null,
    onAfterPrint: null,
    onAuxClick: null,
    onBeforeMatch: null,
    onBeforePrint: null,
    onBeforeToggle: null,
    onBeforeUnload: null,
    onBlur: null,
    onCancel: null,
    onCanPlay: null,
    onCanPlayThrough: null,
    onChange: null,
    onClick: null,
    onClose: null,
    onContextLost: null,
    onContextMenu: null,
    onContextRestored: null,
    onCopy: null,
    onCueChange: null,
    onCut: null,
    onDblClick: null,
    onDrag: null,
    onDragEnd: null,
    onDragEnter: null,
    onDragExit: null,
    onDragLeave: null,
    onDragOver: null,
    onDragStart: null,
    onDrop: null,
    onDurationChange: null,
    onEmptied: null,
    onEnded: null,
    onError: null,
    onFocus: null,
    onFormData: null,
    onHashChange: null,
    onInput: null,
    onInvalid: null,
    onKeyDown: null,
    onKeyPress: null,
    onKeyUp: null,
    onLanguageChange: null,
    onLoad: null,
    onLoadedData: null,
    onLoadedMetadata: null,
    onLoadEnd: null,
    onLoadStart: null,
    onMessage: null,
    onMessageError: null,
    onMouseDown: null,
    onMouseEnter: null,
    onMouseLeave: null,
    onMouseMove: null,
    onMouseOut: null,
    onMouseOver: null,
    onMouseUp: null,
    onOffline: null,
    onOnline: null,
    onPageHide: null,
    onPageShow: null,
    onPaste: null,
    onPause: null,
    onPlay: null,
    onPlaying: null,
    onPopState: null,
    onProgress: null,
    onRateChange: null,
    onRejectionHandled: null,
    onReset: null,
    onResize: null,
    onScroll: null,
    onScrollEnd: null,
    onSecurityPolicyViolation: null,
    onSeeked: null,
    onSeeking: null,
    onSelect: null,
    onSlotChange: null,
    onStalled: null,
    onStorage: null,
    onSubmit: null,
    onSuspend: null,
    onTimeUpdate: null,
    onToggle: null,
    onUnhandledRejection: null,
    onUnload: null,
    onVolumeChange: null,
    onWaiting: null,
    onWheel: null,
    open: boolean,
    optimum: number,
    pattern: null,
    ping: spaceSeparated,
    placeholder: null,
    playsInline: boolean,
    popover: null,
    popoverTarget: null,
    popoverTargetAction: null,
    poster: null,
    preload: null,
    readOnly: boolean,
    referrerPolicy: null,
    rel: spaceSeparated,
    required: boolean,
    reversed: boolean,
    rows: number,
    rowSpan: number,
    sandbox: spaceSeparated,
    scope: null,
    scoped: boolean,
    seamless: boolean,
    selected: boolean,
    shadowRootClonable: boolean,
    shadowRootDelegatesFocus: boolean,
    shadowRootMode: null,
    shape: null,
    size: number,
    sizes: null,
    slot: null,
    span: number,
    spellCheck: booleanish,
    src: null,
    srcDoc: null,
    srcLang: null,
    srcSet: null,
    start: number,
    step: null,
    style: null,
    tabIndex: number,
    target: null,
    title: null,
    translate: null,
    type: null,
    typeMustMatch: boolean,
    useMap: null,
    value: booleanish,
    width: number,
    wrap: null,
    writingSuggestions: null,
    // Legacy.
    // See: https://html.spec.whatwg.org/#other-elements,-attributes-and-apis
    align: null,
    // Several. Use CSS `text-align` instead,
    aLink: null,
    // `<body>`. Use CSS `a:active {color}` instead
    archive: spaceSeparated,
    // `<object>`. List of URIs to archives
    axis: null,
    // `<td>` and `<th>`. Use `scope` on `<th>`
    background: null,
    // `<body>`. Use CSS `background-image` instead
    bgColor: null,
    // `<body>` and table elements. Use CSS `background-color` instead
    border: number,
    // `<table>`. Use CSS `border-width` instead,
    borderColor: null,
    // `<table>`. Use CSS `border-color` instead,
    bottomMargin: number,
    // `<body>`
    cellPadding: null,
    // `<table>`
    cellSpacing: null,
    // `<table>`
    char: null,
    // Several table elements. When `align=char`, sets the character to align on
    charOff: null,
    // Several table elements. When `char`, offsets the alignment
    classId: null,
    // `<object>`
    clear: null,
    // `<br>`. Use CSS `clear` instead
    code: null,
    // `<object>`
    codeBase: null,
    // `<object>`
    codeType: null,
    // `<object>`
    color: null,
    // `<font>` and `<hr>`. Use CSS instead
    compact: boolean,
    // Lists. Use CSS to reduce space between items instead
    declare: boolean,
    // `<object>`
    event: null,
    // `<script>`
    face: null,
    // `<font>`. Use CSS instead
    frame: null,
    // `<table>`
    frameBorder: null,
    // `<iframe>`. Use CSS `border` instead
    hSpace: number,
    // `<img>` and `<object>`
    leftMargin: number,
    // `<body>`
    link: null,
    // `<body>`. Use CSS `a:link {color: *}` instead
    longDesc: null,
    // `<frame>`, `<iframe>`, and `<img>`. Use an `<a>`
    lowSrc: null,
    // `<img>`. Use a `<picture>`
    marginHeight: number,
    // `<body>`
    marginWidth: number,
    // `<body>`
    noResize: boolean,
    // `<frame>`
    noHref: boolean,
    // `<area>`. Use no href instead of an explicit `nohref`
    noShade: boolean,
    // `<hr>`. Use background-color and height instead of borders
    noWrap: boolean,
    // `<td>` and `<th>`
    object: null,
    // `<applet>`
    profile: null,
    // `<head>`
    prompt: null,
    // `<isindex>`
    rev: null,
    // `<link>`
    rightMargin: number,
    // `<body>`
    rules: null,
    // `<table>`
    scheme: null,
    // `<meta>`
    scrolling: booleanish,
    // `<frame>`. Use overflow in the child context
    standby: null,
    // `<object>`
    summary: null,
    // `<table>`
    text: null,
    // `<body>`. Use CSS `color` instead
    topMargin: number,
    // `<body>`
    valueType: null,
    // `<param>`
    version: null,
    // `<html>`. Use a doctype.
    vAlign: null,
    // Several. Use CSS `vertical-align` instead
    vLink: null,
    // `<body>`. Use CSS `a:visited {color}` instead
    vSpace: number,
    // `<img>` and `<object>`
    // Non-standard Properties.
    allowTransparency: null,
    autoCorrect: null,
    autoSave: null,
    disablePictureInPicture: boolean,
    disableRemotePlayback: boolean,
    prefix: null,
    property: null,
    results: number,
    security: null,
    unselectable: null
  },
  space: "html",
  transform: caseInsensitiveTransform
});
const svg$1 = create({
  attributes: {
    accentHeight: "accent-height",
    alignmentBaseline: "alignment-baseline",
    arabicForm: "arabic-form",
    baselineShift: "baseline-shift",
    capHeight: "cap-height",
    className: "class",
    clipPath: "clip-path",
    clipRule: "clip-rule",
    colorInterpolation: "color-interpolation",
    colorInterpolationFilters: "color-interpolation-filters",
    colorProfile: "color-profile",
    colorRendering: "color-rendering",
    crossOrigin: "crossorigin",
    dataType: "datatype",
    dominantBaseline: "dominant-baseline",
    enableBackground: "enable-background",
    fillOpacity: "fill-opacity",
    fillRule: "fill-rule",
    floodColor: "flood-color",
    floodOpacity: "flood-opacity",
    fontFamily: "font-family",
    fontSize: "font-size",
    fontSizeAdjust: "font-size-adjust",
    fontStretch: "font-stretch",
    fontStyle: "font-style",
    fontVariant: "font-variant",
    fontWeight: "font-weight",
    glyphName: "glyph-name",
    glyphOrientationHorizontal: "glyph-orientation-horizontal",
    glyphOrientationVertical: "glyph-orientation-vertical",
    hrefLang: "hreflang",
    horizAdvX: "horiz-adv-x",
    horizOriginX: "horiz-origin-x",
    horizOriginY: "horiz-origin-y",
    imageRendering: "image-rendering",
    letterSpacing: "letter-spacing",
    lightingColor: "lighting-color",
    markerEnd: "marker-end",
    markerMid: "marker-mid",
    markerStart: "marker-start",
    navDown: "nav-down",
    navDownLeft: "nav-down-left",
    navDownRight: "nav-down-right",
    navLeft: "nav-left",
    navNext: "nav-next",
    navPrev: "nav-prev",
    navRight: "nav-right",
    navUp: "nav-up",
    navUpLeft: "nav-up-left",
    navUpRight: "nav-up-right",
    onAbort: "onabort",
    onActivate: "onactivate",
    onAfterPrint: "onafterprint",
    onBeforePrint: "onbeforeprint",
    onBegin: "onbegin",
    onCancel: "oncancel",
    onCanPlay: "oncanplay",
    onCanPlayThrough: "oncanplaythrough",
    onChange: "onchange",
    onClick: "onclick",
    onClose: "onclose",
    onCopy: "oncopy",
    onCueChange: "oncuechange",
    onCut: "oncut",
    onDblClick: "ondblclick",
    onDrag: "ondrag",
    onDragEnd: "ondragend",
    onDragEnter: "ondragenter",
    onDragExit: "ondragexit",
    onDragLeave: "ondragleave",
    onDragOver: "ondragover",
    onDragStart: "ondragstart",
    onDrop: "ondrop",
    onDurationChange: "ondurationchange",
    onEmptied: "onemptied",
    onEnd: "onend",
    onEnded: "onended",
    onError: "onerror",
    onFocus: "onfocus",
    onFocusIn: "onfocusin",
    onFocusOut: "onfocusout",
    onHashChange: "onhashchange",
    onInput: "oninput",
    onInvalid: "oninvalid",
    onKeyDown: "onkeydown",
    onKeyPress: "onkeypress",
    onKeyUp: "onkeyup",
    onLoad: "onload",
    onLoadedData: "onloadeddata",
    onLoadedMetadata: "onloadedmetadata",
    onLoadStart: "onloadstart",
    onMessage: "onmessage",
    onMouseDown: "onmousedown",
    onMouseEnter: "onmouseenter",
    onMouseLeave: "onmouseleave",
    onMouseMove: "onmousemove",
    onMouseOut: "onmouseout",
    onMouseOver: "onmouseover",
    onMouseUp: "onmouseup",
    onMouseWheel: "onmousewheel",
    onOffline: "onoffline",
    onOnline: "ononline",
    onPageHide: "onpagehide",
    onPageShow: "onpageshow",
    onPaste: "onpaste",
    onPause: "onpause",
    onPlay: "onplay",
    onPlaying: "onplaying",
    onPopState: "onpopstate",
    onProgress: "onprogress",
    onRateChange: "onratechange",
    onRepeat: "onrepeat",
    onReset: "onreset",
    onResize: "onresize",
    onScroll: "onscroll",
    onSeeked: "onseeked",
    onSeeking: "onseeking",
    onSelect: "onselect",
    onShow: "onshow",
    onStalled: "onstalled",
    onStorage: "onstorage",
    onSubmit: "onsubmit",
    onSuspend: "onsuspend",
    onTimeUpdate: "ontimeupdate",
    onToggle: "ontoggle",
    onUnload: "onunload",
    onVolumeChange: "onvolumechange",
    onWaiting: "onwaiting",
    onZoom: "onzoom",
    overlinePosition: "overline-position",
    overlineThickness: "overline-thickness",
    paintOrder: "paint-order",
    panose1: "panose-1",
    pointerEvents: "pointer-events",
    referrerPolicy: "referrerpolicy",
    renderingIntent: "rendering-intent",
    shapeRendering: "shape-rendering",
    stopColor: "stop-color",
    stopOpacity: "stop-opacity",
    strikethroughPosition: "strikethrough-position",
    strikethroughThickness: "strikethrough-thickness",
    strokeDashArray: "stroke-dasharray",
    strokeDashOffset: "stroke-dashoffset",
    strokeLineCap: "stroke-linecap",
    strokeLineJoin: "stroke-linejoin",
    strokeMiterLimit: "stroke-miterlimit",
    strokeOpacity: "stroke-opacity",
    strokeWidth: "stroke-width",
    tabIndex: "tabindex",
    textAnchor: "text-anchor",
    textDecoration: "text-decoration",
    textRendering: "text-rendering",
    transformOrigin: "transform-origin",
    typeOf: "typeof",
    underlinePosition: "underline-position",
    underlineThickness: "underline-thickness",
    unicodeBidi: "unicode-bidi",
    unicodeRange: "unicode-range",
    unitsPerEm: "units-per-em",
    vAlphabetic: "v-alphabetic",
    vHanging: "v-hanging",
    vIdeographic: "v-ideographic",
    vMathematical: "v-mathematical",
    vectorEffect: "vector-effect",
    vertAdvY: "vert-adv-y",
    vertOriginX: "vert-origin-x",
    vertOriginY: "vert-origin-y",
    wordSpacing: "word-spacing",
    writingMode: "writing-mode",
    xHeight: "x-height",
    // These were camelcased in Tiny. Now lowercased in SVG 2
    playbackOrder: "playbackorder",
    timelineBegin: "timelinebegin"
  },
  properties: {
    about: commaOrSpaceSeparated,
    accentHeight: number,
    accumulate: null,
    additive: null,
    alignmentBaseline: null,
    alphabetic: number,
    amplitude: number,
    arabicForm: null,
    ascent: number,
    attributeName: null,
    attributeType: null,
    azimuth: number,
    bandwidth: null,
    baselineShift: null,
    baseFrequency: null,
    baseProfile: null,
    bbox: null,
    begin: null,
    bias: number,
    by: null,
    calcMode: null,
    capHeight: number,
    className: spaceSeparated,
    clip: null,
    clipPath: null,
    clipPathUnits: null,
    clipRule: null,
    color: null,
    colorInterpolation: null,
    colorInterpolationFilters: null,
    colorProfile: null,
    colorRendering: null,
    content: null,
    contentScriptType: null,
    contentStyleType: null,
    crossOrigin: null,
    cursor: null,
    cx: null,
    cy: null,
    d: null,
    dataType: null,
    defaultAction: null,
    descent: number,
    diffuseConstant: number,
    direction: null,
    display: null,
    dur: null,
    divisor: number,
    dominantBaseline: null,
    download: boolean,
    dx: null,
    dy: null,
    edgeMode: null,
    editable: null,
    elevation: number,
    enableBackground: null,
    end: null,
    event: null,
    exponent: number,
    externalResourcesRequired: null,
    fill: null,
    fillOpacity: number,
    fillRule: null,
    filter: null,
    filterRes: null,
    filterUnits: null,
    floodColor: null,
    floodOpacity: null,
    focusable: null,
    focusHighlight: null,
    fontFamily: null,
    fontSize: null,
    fontSizeAdjust: null,
    fontStretch: null,
    fontStyle: null,
    fontVariant: null,
    fontWeight: null,
    format: null,
    fr: null,
    from: null,
    fx: null,
    fy: null,
    g1: commaSeparated,
    g2: commaSeparated,
    glyphName: commaSeparated,
    glyphOrientationHorizontal: null,
    glyphOrientationVertical: null,
    glyphRef: null,
    gradientTransform: null,
    gradientUnits: null,
    handler: null,
    hanging: number,
    hatchContentUnits: null,
    hatchUnits: null,
    height: null,
    href: null,
    hrefLang: null,
    horizAdvX: number,
    horizOriginX: number,
    horizOriginY: number,
    id: null,
    ideographic: number,
    imageRendering: null,
    initialVisibility: null,
    in: null,
    in2: null,
    intercept: number,
    k: number,
    k1: number,
    k2: number,
    k3: number,
    k4: number,
    kernelMatrix: commaOrSpaceSeparated,
    kernelUnitLength: null,
    keyPoints: null,
    // SEMI_COLON_SEPARATED
    keySplines: null,
    // SEMI_COLON_SEPARATED
    keyTimes: null,
    // SEMI_COLON_SEPARATED
    kerning: null,
    lang: null,
    lengthAdjust: null,
    letterSpacing: null,
    lightingColor: null,
    limitingConeAngle: number,
    local: null,
    markerEnd: null,
    markerMid: null,
    markerStart: null,
    markerHeight: null,
    markerUnits: null,
    markerWidth: null,
    mask: null,
    maskContentUnits: null,
    maskUnits: null,
    mathematical: null,
    max: null,
    media: null,
    mediaCharacterEncoding: null,
    mediaContentEncodings: null,
    mediaSize: number,
    mediaTime: null,
    method: null,
    min: null,
    mode: null,
    name: null,
    navDown: null,
    navDownLeft: null,
    navDownRight: null,
    navLeft: null,
    navNext: null,
    navPrev: null,
    navRight: null,
    navUp: null,
    navUpLeft: null,
    navUpRight: null,
    numOctaves: null,
    observer: null,
    offset: null,
    onAbort: null,
    onActivate: null,
    onAfterPrint: null,
    onBeforePrint: null,
    onBegin: null,
    onCancel: null,
    onCanPlay: null,
    onCanPlayThrough: null,
    onChange: null,
    onClick: null,
    onClose: null,
    onCopy: null,
    onCueChange: null,
    onCut: null,
    onDblClick: null,
    onDrag: null,
    onDragEnd: null,
    onDragEnter: null,
    onDragExit: null,
    onDragLeave: null,
    onDragOver: null,
    onDragStart: null,
    onDrop: null,
    onDurationChange: null,
    onEmptied: null,
    onEnd: null,
    onEnded: null,
    onError: null,
    onFocus: null,
    onFocusIn: null,
    onFocusOut: null,
    onHashChange: null,
    onInput: null,
    onInvalid: null,
    onKeyDown: null,
    onKeyPress: null,
    onKeyUp: null,
    onLoad: null,
    onLoadedData: null,
    onLoadedMetadata: null,
    onLoadStart: null,
    onMessage: null,
    onMouseDown: null,
    onMouseEnter: null,
    onMouseLeave: null,
    onMouseMove: null,
    onMouseOut: null,
    onMouseOver: null,
    onMouseUp: null,
    onMouseWheel: null,
    onOffline: null,
    onOnline: null,
    onPageHide: null,
    onPageShow: null,
    onPaste: null,
    onPause: null,
    onPlay: null,
    onPlaying: null,
    onPopState: null,
    onProgress: null,
    onRateChange: null,
    onRepeat: null,
    onReset: null,
    onResize: null,
    onScroll: null,
    onSeeked: null,
    onSeeking: null,
    onSelect: null,
    onShow: null,
    onStalled: null,
    onStorage: null,
    onSubmit: null,
    onSuspend: null,
    onTimeUpdate: null,
    onToggle: null,
    onUnload: null,
    onVolumeChange: null,
    onWaiting: null,
    onZoom: null,
    opacity: null,
    operator: null,
    order: null,
    orient: null,
    orientation: null,
    origin: null,
    overflow: null,
    overlay: null,
    overlinePosition: number,
    overlineThickness: number,
    paintOrder: null,
    panose1: null,
    path: null,
    pathLength: number,
    patternContentUnits: null,
    patternTransform: null,
    patternUnits: null,
    phase: null,
    ping: spaceSeparated,
    pitch: null,
    playbackOrder: null,
    pointerEvents: null,
    points: null,
    pointsAtX: number,
    pointsAtY: number,
    pointsAtZ: number,
    preserveAlpha: null,
    preserveAspectRatio: null,
    primitiveUnits: null,
    propagate: null,
    property: commaOrSpaceSeparated,
    r: null,
    radius: null,
    referrerPolicy: null,
    refX: null,
    refY: null,
    rel: commaOrSpaceSeparated,
    rev: commaOrSpaceSeparated,
    renderingIntent: null,
    repeatCount: null,
    repeatDur: null,
    requiredExtensions: commaOrSpaceSeparated,
    requiredFeatures: commaOrSpaceSeparated,
    requiredFonts: commaOrSpaceSeparated,
    requiredFormats: commaOrSpaceSeparated,
    resource: null,
    restart: null,
    result: null,
    rotate: null,
    rx: null,
    ry: null,
    scale: null,
    seed: null,
    shapeRendering: null,
    side: null,
    slope: null,
    snapshotTime: null,
    specularConstant: number,
    specularExponent: number,
    spreadMethod: null,
    spacing: null,
    startOffset: null,
    stdDeviation: null,
    stemh: null,
    stemv: null,
    stitchTiles: null,
    stopColor: null,
    stopOpacity: null,
    strikethroughPosition: number,
    strikethroughThickness: number,
    string: null,
    stroke: null,
    strokeDashArray: commaOrSpaceSeparated,
    strokeDashOffset: null,
    strokeLineCap: null,
    strokeLineJoin: null,
    strokeMiterLimit: number,
    strokeOpacity: number,
    strokeWidth: null,
    style: null,
    surfaceScale: number,
    syncBehavior: null,
    syncBehaviorDefault: null,
    syncMaster: null,
    syncTolerance: null,
    syncToleranceDefault: null,
    systemLanguage: commaOrSpaceSeparated,
    tabIndex: number,
    tableValues: null,
    target: null,
    targetX: number,
    targetY: number,
    textAnchor: null,
    textDecoration: null,
    textRendering: null,
    textLength: null,
    timelineBegin: null,
    title: null,
    transformBehavior: null,
    type: null,
    typeOf: commaOrSpaceSeparated,
    to: null,
    transform: null,
    transformOrigin: null,
    u1: null,
    u2: null,
    underlinePosition: number,
    underlineThickness: number,
    unicode: null,
    unicodeBidi: null,
    unicodeRange: null,
    unitsPerEm: number,
    values: null,
    vAlphabetic: number,
    vMathematical: number,
    vectorEffect: null,
    vHanging: number,
    vIdeographic: number,
    version: null,
    vertAdvY: number,
    vertOriginX: number,
    vertOriginY: number,
    viewBox: null,
    viewTarget: null,
    visibility: null,
    width: null,
    widths: null,
    wordSpacing: null,
    writingMode: null,
    x: null,
    x1: null,
    x2: null,
    xChannelSelector: null,
    xHeight: number,
    y: null,
    y1: null,
    y2: null,
    yChannelSelector: null,
    z: null,
    zoomAndPan: null
  },
  space: "svg",
  transform: caseSensitiveTransform
});
const xlink = create({
  properties: {
    xLinkActuate: null,
    xLinkArcRole: null,
    xLinkHref: null,
    xLinkRole: null,
    xLinkShow: null,
    xLinkTitle: null,
    xLinkType: null
  },
  space: "xlink",
  transform(_2, property) {
    return "xlink:" + property.slice(5).toLowerCase();
  }
});
const xmlns = create({
  attributes: { xmlnsxlink: "xmlns:xlink" },
  properties: { xmlnsXLink: null, xmlns: null },
  space: "xmlns",
  transform: caseInsensitiveTransform
});
const xml = create({
  properties: { xmlBase: null, xmlLang: null, xmlSpace: null },
  space: "xml",
  transform(_2, property) {
    return "xml:" + property.slice(3).toLowerCase();
  }
});
const cap = /[A-Z]/g;
const dash = /-[a-z]/g;
const valid = /^data[-\w.:]+$/i;
function find(schema, value) {
  const normal = normalize(value);
  let property = value;
  let Type = Info;
  if (normal in schema.normal) {
    return schema.property[schema.normal[normal]];
  }
  if (normal.length > 4 && normal.slice(0, 4) === "data" && valid.test(value)) {
    if (value.charAt(4) === "-") {
      const rest = value.slice(5).replace(dash, camelcase);
      property = "data" + rest.charAt(0).toUpperCase() + rest.slice(1);
    } else {
      const rest = value.slice(4);
      if (!dash.test(rest)) {
        let dashes = rest.replace(cap, kebab);
        if (dashes.charAt(0) !== "-") {
          dashes = "-" + dashes;
        }
        value = "data" + dashes;
      }
    }
    Type = DefinedInfo;
  }
  return new Type(property, value);
}
function kebab($0) {
  return "-" + $0.toLowerCase();
}
function camelcase($0) {
  return $0.charAt(1).toUpperCase();
}
const html$2 = merge([aria, html$3, xlink, xmlns, xml], "html");
const svg = merge([aria, svg$1, xlink, xmlns, xml], "svg");
const own$2 = {}.hasOwnProperty;
function zwitch(key2, options) {
  const settings = options || {};
  function one2(value, ...parameters) {
    let fn = one2.invalid;
    const handlers = one2.handlers;
    if (value && own$2.call(value, key2)) {
      const id = String(value[key2]);
      fn = own$2.call(handlers, id) ? handlers[id] : one2.unknown;
    }
    if (fn) {
      return fn.call(this, value, ...parameters);
    }
  }
  one2.handlers = settings.handlers || {};
  one2.invalid = settings.invalid;
  one2.unknown = settings.unknown;
  return one2;
}
const defaultSubsetRegex = /["&'<>`]/g;
const surrogatePairsRegex = /[\uD800-\uDBFF][\uDC00-\uDFFF]/g;
const controlCharactersRegex = (
  // eslint-disable-next-line no-control-regex, unicorn/no-hex-escape
  /[\x01-\t\v\f\x0E-\x1F\x7F\x81\x8D\x8F\x90\x9D\xA0-\uFFFF]/g
);
const regexEscapeRegex = /[|\\{}()[\]^$+*?.]/g;
const subsetToRegexCache = /* @__PURE__ */ new WeakMap();
function core(value, options) {
  value = value.replace(
    options.subset ? charactersToExpressionCached(options.subset) : defaultSubsetRegex,
    basic
  );
  if (options.subset || options.escapeOnly) {
    return value;
  }
  return value.replace(surrogatePairsRegex, surrogate).replace(controlCharactersRegex, basic);
  function surrogate(pair, index, all2) {
    return options.format(
      (pair.charCodeAt(0) - 55296) * 1024 + pair.charCodeAt(1) - 56320 + 65536,
      all2.charCodeAt(index + 2),
      options
    );
  }
  function basic(character, index, all2) {
    return options.format(
      character.charCodeAt(0),
      all2.charCodeAt(index + 1),
      options
    );
  }
}
function charactersToExpressionCached(subset) {
  let cached = subsetToRegexCache.get(subset);
  if (!cached) {
    cached = charactersToExpression(subset);
    subsetToRegexCache.set(subset, cached);
  }
  return cached;
}
function charactersToExpression(subset) {
  const groups = [];
  let index = -1;
  while (++index < subset.length) {
    groups.push(subset[index].replace(regexEscapeRegex, "\\$&"));
  }
  return new RegExp("(?:" + groups.join("|") + ")", "g");
}
const hexadecimalRegex = /[\dA-Fa-f]/;
function toHexadecimal(code, next, omit) {
  const value = "&#x" + code.toString(16).toUpperCase();
  return omit && next && !hexadecimalRegex.test(String.fromCharCode(next)) ? value : value + ";";
}
const decimalRegex = /\d/;
function toDecimal(code, next, omit) {
  const value = "&#" + String(code);
  return omit && next && !decimalRegex.test(String.fromCharCode(next)) ? value : value + ";";
}
const characterEntitiesLegacy = [
  "AElig",
  "AMP",
  "Aacute",
  "Acirc",
  "Agrave",
  "Aring",
  "Atilde",
  "Auml",
  "COPY",
  "Ccedil",
  "ETH",
  "Eacute",
  "Ecirc",
  "Egrave",
  "Euml",
  "GT",
  "Iacute",
  "Icirc",
  "Igrave",
  "Iuml",
  "LT",
  "Ntilde",
  "Oacute",
  "Ocirc",
  "Ograve",
  "Oslash",
  "Otilde",
  "Ouml",
  "QUOT",
  "REG",
  "THORN",
  "Uacute",
  "Ucirc",
  "Ugrave",
  "Uuml",
  "Yacute",
  "aacute",
  "acirc",
  "acute",
  "aelig",
  "agrave",
  "amp",
  "aring",
  "atilde",
  "auml",
  "brvbar",
  "ccedil",
  "cedil",
  "cent",
  "copy",
  "curren",
  "deg",
  "divide",
  "eacute",
  "ecirc",
  "egrave",
  "eth",
  "euml",
  "frac12",
  "frac14",
  "frac34",
  "gt",
  "iacute",
  "icirc",
  "iexcl",
  "igrave",
  "iquest",
  "iuml",
  "laquo",
  "lt",
  "macr",
  "micro",
  "middot",
  "nbsp",
  "not",
  "ntilde",
  "oacute",
  "ocirc",
  "ograve",
  "ordf",
  "ordm",
  "oslash",
  "otilde",
  "ouml",
  "para",
  "plusmn",
  "pound",
  "quot",
  "raquo",
  "reg",
  "sect",
  "shy",
  "sup1",
  "sup2",
  "sup3",
  "szlig",
  "thorn",
  "times",
  "uacute",
  "ucirc",
  "ugrave",
  "uml",
  "uuml",
  "yacute",
  "yen",
  "yuml"
];
const characterEntitiesHtml4 = {
  nbsp: " ",
  iexcl: "¡",
  cent: "¢",
  pound: "£",
  curren: "¤",
  yen: "¥",
  brvbar: "¦",
  sect: "§",
  uml: "¨",
  copy: "©",
  ordf: "ª",
  laquo: "«",
  not: "¬",
  shy: "­",
  reg: "®",
  macr: "¯",
  deg: "°",
  plusmn: "±",
  sup2: "²",
  sup3: "³",
  acute: "´",
  micro: "µ",
  para: "¶",
  middot: "·",
  cedil: "¸",
  sup1: "¹",
  ordm: "º",
  raquo: "»",
  frac14: "¼",
  frac12: "½",
  frac34: "¾",
  iquest: "¿",
  Agrave: "À",
  Aacute: "Á",
  Acirc: "Â",
  Atilde: "Ã",
  Auml: "Ä",
  Aring: "Å",
  AElig: "Æ",
  Ccedil: "Ç",
  Egrave: "È",
  Eacute: "É",
  Ecirc: "Ê",
  Euml: "Ë",
  Igrave: "Ì",
  Iacute: "Í",
  Icirc: "Î",
  Iuml: "Ï",
  ETH: "Ð",
  Ntilde: "Ñ",
  Ograve: "Ò",
  Oacute: "Ó",
  Ocirc: "Ô",
  Otilde: "Õ",
  Ouml: "Ö",
  times: "×",
  Oslash: "Ø",
  Ugrave: "Ù",
  Uacute: "Ú",
  Ucirc: "Û",
  Uuml: "Ü",
  Yacute: "Ý",
  THORN: "Þ",
  szlig: "ß",
  agrave: "à",
  aacute: "á",
  acirc: "â",
  atilde: "ã",
  auml: "ä",
  aring: "å",
  aelig: "æ",
  ccedil: "ç",
  egrave: "è",
  eacute: "é",
  ecirc: "ê",
  euml: "ë",
  igrave: "ì",
  iacute: "í",
  icirc: "î",
  iuml: "ï",
  eth: "ð",
  ntilde: "ñ",
  ograve: "ò",
  oacute: "ó",
  ocirc: "ô",
  otilde: "õ",
  ouml: "ö",
  divide: "÷",
  oslash: "ø",
  ugrave: "ù",
  uacute: "ú",
  ucirc: "û",
  uuml: "ü",
  yacute: "ý",
  thorn: "þ",
  yuml: "ÿ",
  fnof: "ƒ",
  Alpha: "Α",
  Beta: "Β",
  Gamma: "Γ",
  Delta: "Δ",
  Epsilon: "Ε",
  Zeta: "Ζ",
  Eta: "Η",
  Theta: "Θ",
  Iota: "Ι",
  Kappa: "Κ",
  Lambda: "Λ",
  Mu: "Μ",
  Nu: "Ν",
  Xi: "Ξ",
  Omicron: "Ο",
  Pi: "Π",
  Rho: "Ρ",
  Sigma: "Σ",
  Tau: "Τ",
  Upsilon: "Υ",
  Phi: "Φ",
  Chi: "Χ",
  Psi: "Ψ",
  Omega: "Ω",
  alpha: "α",
  beta: "β",
  gamma: "γ",
  delta: "δ",
  epsilon: "ε",
  zeta: "ζ",
  eta: "η",
  theta: "θ",
  iota: "ι",
  kappa: "κ",
  lambda: "λ",
  mu: "μ",
  nu: "ν",
  xi: "ξ",
  omicron: "ο",
  pi: "π",
  rho: "ρ",
  sigmaf: "ς",
  sigma: "σ",
  tau: "τ",
  upsilon: "υ",
  phi: "φ",
  chi: "χ",
  psi: "ψ",
  omega: "ω",
  thetasym: "ϑ",
  upsih: "ϒ",
  piv: "ϖ",
  bull: "•",
  hellip: "…",
  prime: "′",
  Prime: "″",
  oline: "‾",
  frasl: "⁄",
  weierp: "℘",
  image: "ℑ",
  real: "ℜ",
  trade: "™",
  alefsym: "ℵ",
  larr: "←",
  uarr: "↑",
  rarr: "→",
  darr: "↓",
  harr: "↔",
  crarr: "↵",
  lArr: "⇐",
  uArr: "⇑",
  rArr: "⇒",
  dArr: "⇓",
  hArr: "⇔",
  forall: "∀",
  part: "∂",
  exist: "∃",
  empty: "∅",
  nabla: "∇",
  isin: "∈",
  notin: "∉",
  ni: "∋",
  prod: "∏",
  sum: "∑",
  minus: "−",
  lowast: "∗",
  radic: "√",
  prop: "∝",
  infin: "∞",
  ang: "∠",
  and: "∧",
  or: "∨",
  cap: "∩",
  cup: "∪",
  int: "∫",
  there4: "∴",
  sim: "∼",
  cong: "≅",
  asymp: "≈",
  ne: "≠",
  equiv: "≡",
  le: "≤",
  ge: "≥",
  sub: "⊂",
  sup: "⊃",
  nsub: "⊄",
  sube: "⊆",
  supe: "⊇",
  oplus: "⊕",
  otimes: "⊗",
  perp: "⊥",
  sdot: "⋅",
  lceil: "⌈",
  rceil: "⌉",
  lfloor: "⌊",
  rfloor: "⌋",
  lang: "〈",
  rang: "〉",
  loz: "◊",
  spades: "♠",
  clubs: "♣",
  hearts: "♥",
  diams: "♦",
  quot: '"',
  amp: "&",
  lt: "<",
  gt: ">",
  OElig: "Œ",
  oelig: "œ",
  Scaron: "Š",
  scaron: "š",
  Yuml: "Ÿ",
  circ: "ˆ",
  tilde: "˜",
  ensp: " ",
  emsp: " ",
  thinsp: " ",
  zwnj: "‌",
  zwj: "‍",
  lrm: "‎",
  rlm: "‏",
  ndash: "–",
  mdash: "—",
  lsquo: "‘",
  rsquo: "’",
  sbquo: "‚",
  ldquo: "“",
  rdquo: "”",
  bdquo: "„",
  dagger: "†",
  Dagger: "‡",
  permil: "‰",
  lsaquo: "‹",
  rsaquo: "›",
  euro: "€"
};
const dangerous = [
  "cent",
  "copy",
  "divide",
  "gt",
  "lt",
  "not",
  "para",
  "times"
];
const own$1 = {}.hasOwnProperty;
const characters = {};
let key;
for (key in characterEntitiesHtml4) {
  if (own$1.call(characterEntitiesHtml4, key)) {
    characters[characterEntitiesHtml4[key]] = key;
  }
}
const notAlphanumericRegex = /[^\dA-Za-z]/;
function toNamed(code, next, omit, attribute) {
  const character = String.fromCharCode(code);
  if (own$1.call(characters, character)) {
    const name = characters[character];
    const value = "&" + name;
    if (omit && characterEntitiesLegacy.includes(name) && !dangerous.includes(name) && (!attribute || next && next !== 61 && notAlphanumericRegex.test(String.fromCharCode(next)))) {
      return value;
    }
    return value + ";";
  }
  return "";
}
function formatSmart(code, next, options) {
  let numeric = toHexadecimal(code, next, options.omitOptionalSemicolons);
  let named;
  if (options.useNamedReferences || options.useShortestReferences) {
    named = toNamed(
      code,
      next,
      options.omitOptionalSemicolons,
      options.attribute
    );
  }
  if ((options.useShortestReferences || !named) && options.useShortestReferences) {
    const decimal = toDecimal(code, next, options.omitOptionalSemicolons);
    if (decimal.length < numeric.length) {
      numeric = decimal;
    }
  }
  return named && (!options.useShortestReferences || named.length < numeric.length) ? named : numeric;
}
function stringifyEntities(value, options) {
  return core(value, Object.assign({ format: formatSmart }, options));
}
const htmlCommentRegex = /^>|^->|<!--|-->|--!>|<!-$/g;
const bogusCommentEntitySubset = [">"];
const commentEntitySubset = ["<", ">"];
function comment(node, _1, _2, state) {
  return state.settings.bogusComments ? "<?" + stringifyEntities(
    node.value,
    Object.assign({}, state.settings.characterReferences, {
      subset: bogusCommentEntitySubset
    })
  ) + ">" : "<!--" + node.value.replace(htmlCommentRegex, encode) + "-->";
  function encode($0) {
    return stringifyEntities(
      $0,
      Object.assign({}, state.settings.characterReferences, {
        subset: commentEntitySubset
      })
    );
  }
}
function doctype(_1, _2, _3, state) {
  return "<!" + (state.settings.upperDoctype ? "DOCTYPE" : "doctype") + (state.settings.tightDoctype ? "" : " ") + "html>";
}
function ccount(value, character) {
  const source = String(value);
  if (typeof character !== "string") {
    throw new TypeError("Expected character");
  }
  let count = 0;
  let index = source.indexOf(character);
  while (index !== -1) {
    count++;
    index = source.indexOf(character, index + character.length);
  }
  return count;
}
function stringify$2(values, options) {
  const settings = options || {};
  const input = values[values.length - 1] === "" ? [...values, ""] : values;
  return input.join(
    (settings.padRight ? " " : "") + "," + (settings.padLeft === false ? "" : " ")
  ).trim();
}
function stringify$1(values) {
  return values.join(" ").trim();
}
const re$1 = /[ \t\n\f\r]/g;
function whitespace(thing) {
  return typeof thing === "object" ? thing.type === "text" ? empty(thing.value) : false : empty(thing);
}
function empty(value) {
  return value.replace(re$1, "") === "";
}
const siblingAfter = siblings(1);
const siblingBefore = siblings(-1);
const emptyChildren$1 = [];
function siblings(increment2) {
  return sibling;
  function sibling(parent, index, includeWhitespace) {
    const siblings2 = parent ? parent.children : emptyChildren$1;
    let offset = (index || 0) + increment2;
    let next = siblings2[offset];
    if (!includeWhitespace) {
      while (next && whitespace(next)) {
        offset += increment2;
        next = siblings2[offset];
      }
    }
    return next;
  }
}
const own = {}.hasOwnProperty;
function omission(handlers) {
  return omit;
  function omit(node, index, parent) {
    return own.call(handlers, node.tagName) && handlers[node.tagName](node, index, parent);
  }
}
const closing = omission({
  body: body$1,
  caption: headOrColgroupOrCaption,
  colgroup: headOrColgroupOrCaption,
  dd,
  dt,
  head: headOrColgroupOrCaption,
  html: html$1,
  li,
  optgroup,
  option,
  p,
  rp: rubyElement,
  rt: rubyElement,
  tbody: tbody$1,
  td: cells,
  tfoot,
  th: cells,
  thead,
  tr
});
function headOrColgroupOrCaption(_2, index, parent) {
  const next = siblingAfter(parent, index, true);
  return !next || next.type !== "comment" && !(next.type === "text" && whitespace(next.value.charAt(0)));
}
function html$1(_2, index, parent) {
  const next = siblingAfter(parent, index);
  return !next || next.type !== "comment";
}
function body$1(_2, index, parent) {
  const next = siblingAfter(parent, index);
  return !next || next.type !== "comment";
}
function p(_2, index, parent) {
  const next = siblingAfter(parent, index);
  return next ? next.type === "element" && (next.tagName === "address" || next.tagName === "article" || next.tagName === "aside" || next.tagName === "blockquote" || next.tagName === "details" || next.tagName === "div" || next.tagName === "dl" || next.tagName === "fieldset" || next.tagName === "figcaption" || next.tagName === "figure" || next.tagName === "footer" || next.tagName === "form" || next.tagName === "h1" || next.tagName === "h2" || next.tagName === "h3" || next.tagName === "h4" || next.tagName === "h5" || next.tagName === "h6" || next.tagName === "header" || next.tagName === "hgroup" || next.tagName === "hr" || next.tagName === "main" || next.tagName === "menu" || next.tagName === "nav" || next.tagName === "ol" || next.tagName === "p" || next.tagName === "pre" || next.tagName === "section" || next.tagName === "table" || next.tagName === "ul") : !parent || // Confusing parent.
  !(parent.type === "element" && (parent.tagName === "a" || parent.tagName === "audio" || parent.tagName === "del" || parent.tagName === "ins" || parent.tagName === "map" || parent.tagName === "noscript" || parent.tagName === "video"));
}
function li(_2, index, parent) {
  const next = siblingAfter(parent, index);
  return !next || next.type === "element" && next.tagName === "li";
}
function dt(_2, index, parent) {
  const next = siblingAfter(parent, index);
  return Boolean(
    next && next.type === "element" && (next.tagName === "dt" || next.tagName === "dd")
  );
}
function dd(_2, index, parent) {
  const next = siblingAfter(parent, index);
  return !next || next.type === "element" && (next.tagName === "dt" || next.tagName === "dd");
}
function rubyElement(_2, index, parent) {
  const next = siblingAfter(parent, index);
  return !next || next.type === "element" && (next.tagName === "rp" || next.tagName === "rt");
}
function optgroup(_2, index, parent) {
  const next = siblingAfter(parent, index);
  return !next || next.type === "element" && next.tagName === "optgroup";
}
function option(_2, index, parent) {
  const next = siblingAfter(parent, index);
  return !next || next.type === "element" && (next.tagName === "option" || next.tagName === "optgroup");
}
function thead(_2, index, parent) {
  const next = siblingAfter(parent, index);
  return Boolean(
    next && next.type === "element" && (next.tagName === "tbody" || next.tagName === "tfoot")
  );
}
function tbody$1(_2, index, parent) {
  const next = siblingAfter(parent, index);
  return !next || next.type === "element" && (next.tagName === "tbody" || next.tagName === "tfoot");
}
function tfoot(_2, index, parent) {
  return !siblingAfter(parent, index);
}
function tr(_2, index, parent) {
  const next = siblingAfter(parent, index);
  return !next || next.type === "element" && next.tagName === "tr";
}
function cells(_2, index, parent) {
  const next = siblingAfter(parent, index);
  return !next || next.type === "element" && (next.tagName === "td" || next.tagName === "th");
}
const opening = omission({
  body,
  colgroup,
  head,
  html,
  tbody
});
function html(node) {
  const head2 = siblingAfter(node, -1);
  return !head2 || head2.type !== "comment";
}
function head(node) {
  const seen = /* @__PURE__ */ new Set();
  for (const child2 of node.children) {
    if (child2.type === "element" && (child2.tagName === "base" || child2.tagName === "title")) {
      if (seen.has(child2.tagName)) return false;
      seen.add(child2.tagName);
    }
  }
  const child = node.children[0];
  return !child || child.type === "element";
}
function body(node) {
  const head2 = siblingAfter(node, -1, true);
  return !head2 || head2.type !== "comment" && !(head2.type === "text" && whitespace(head2.value.charAt(0))) && !(head2.type === "element" && (head2.tagName === "meta" || head2.tagName === "link" || head2.tagName === "script" || head2.tagName === "style" || head2.tagName === "template"));
}
function colgroup(node, index, parent) {
  const previous = siblingBefore(parent, index);
  const head2 = siblingAfter(node, -1, true);
  if (parent && previous && previous.type === "element" && previous.tagName === "colgroup" && closing(previous, parent.children.indexOf(previous), parent)) {
    return false;
  }
  return Boolean(head2 && head2.type === "element" && head2.tagName === "col");
}
function tbody(node, index, parent) {
  const previous = siblingBefore(parent, index);
  const head2 = siblingAfter(node, -1);
  if (parent && previous && previous.type === "element" && (previous.tagName === "thead" || previous.tagName === "tbody") && closing(previous, parent.children.indexOf(previous), parent)) {
    return false;
  }
  return Boolean(head2 && head2.type === "element" && head2.tagName === "tr");
}
const constants = {
  // See: <https://html.spec.whatwg.org/#attribute-name-state>.
  name: [
    ["	\n\f\r &/=>".split(""), "	\n\f\r \"&'/=>`".split("")],
    [`\0	
\f\r "&'/<=>`.split(""), "\0	\n\f\r \"&'/<=>`".split("")]
  ],
  // See: <https://html.spec.whatwg.org/#attribute-value-(unquoted)-state>.
  unquoted: [
    ["	\n\f\r &>".split(""), "\0	\n\f\r \"&'<=>`".split("")],
    ["\0	\n\f\r \"&'<=>`".split(""), "\0	\n\f\r \"&'<=>`".split("")]
  ],
  // See: <https://html.spec.whatwg.org/#attribute-value-(single-quoted)-state>.
  single: [
    ["&'".split(""), "\"&'`".split("")],
    ["\0&'".split(""), "\0\"&'`".split("")]
  ],
  // See: <https://html.spec.whatwg.org/#attribute-value-(double-quoted)-state>.
  double: [
    ['"&'.split(""), "\"&'`".split("")],
    ['\0"&'.split(""), "\0\"&'`".split("")]
  ]
};
function element(node, index, parent, state) {
  const schema = state.schema;
  const omit = schema.space === "svg" ? false : state.settings.omitOptionalTags;
  let selfClosing = schema.space === "svg" ? state.settings.closeEmptyElements : state.settings.voids.includes(node.tagName.toLowerCase());
  const parts = [];
  let last;
  if (schema.space === "html" && node.tagName === "svg") {
    state.schema = svg;
  }
  const attributes = serializeAttributes(state, node.properties);
  const content = state.all(
    schema.space === "html" && node.tagName === "template" ? node.content : node
  );
  state.schema = schema;
  if (content) selfClosing = false;
  if (attributes || !omit || !opening(node, index, parent)) {
    parts.push("<", node.tagName, attributes ? " " + attributes : "");
    if (selfClosing && (schema.space === "svg" || state.settings.closeSelfClosing)) {
      last = attributes.charAt(attributes.length - 1);
      if (!state.settings.tightSelfClosing || last === "/" || last && last !== '"' && last !== "'") {
        parts.push(" ");
      }
      parts.push("/");
    }
    parts.push(">");
  }
  parts.push(content);
  if (!selfClosing && (!omit || !closing(node, index, parent))) {
    parts.push("</" + node.tagName + ">");
  }
  return parts.join("");
}
function serializeAttributes(state, properties) {
  const values = [];
  let index = -1;
  let key2;
  if (properties) {
    for (key2 in properties) {
      if (properties[key2] !== null && properties[key2] !== void 0) {
        const value = serializeAttribute(state, key2, properties[key2]);
        if (value) values.push(value);
      }
    }
  }
  while (++index < values.length) {
    const last = state.settings.tightAttributes ? values[index].charAt(values[index].length - 1) : void 0;
    if (index !== values.length - 1 && last !== '"' && last !== "'") {
      values[index] += " ";
    }
  }
  return values.join("");
}
function serializeAttribute(state, key2, value) {
  const info = find(state.schema, key2);
  const x2 = state.settings.allowParseErrors && state.schema.space === "html" ? 0 : 1;
  const y2 = state.settings.allowDangerousCharacters ? 0 : 1;
  let quote = state.quote;
  let result;
  if (info.overloadedBoolean && (value === info.attribute || value === "")) {
    value = true;
  } else if ((info.boolean || info.overloadedBoolean) && (typeof value !== "string" || value === info.attribute || value === "")) {
    value = Boolean(value);
  }
  if (value === null || value === void 0 || value === false || typeof value === "number" && Number.isNaN(value)) {
    return "";
  }
  const name = stringifyEntities(
    info.attribute,
    Object.assign({}, state.settings.characterReferences, {
      // Always encode without parse errors in non-HTML.
      subset: constants.name[x2][y2]
    })
  );
  if (value === true) return name;
  value = Array.isArray(value) ? (info.commaSeparated ? stringify$2 : stringify$1)(value, {
    padLeft: !state.settings.tightCommaSeparatedLists
  }) : String(value);
  if (state.settings.collapseEmptyAttributes && !value) return name;
  if (state.settings.preferUnquoted) {
    result = stringifyEntities(
      value,
      Object.assign({}, state.settings.characterReferences, {
        attribute: true,
        subset: constants.unquoted[x2][y2]
      })
    );
  }
  if (result !== value) {
    if (state.settings.quoteSmart && ccount(value, quote) > ccount(value, state.alternative)) {
      quote = state.alternative;
    }
    result = quote + stringifyEntities(
      value,
      Object.assign({}, state.settings.characterReferences, {
        // Always encode without parse errors in non-HTML.
        subset: (quote === "'" ? constants.single : constants.double)[x2][y2],
        attribute: true
      })
    ) + quote;
  }
  return name + (result ? "=" + result : result);
}
const textEntitySubset = ["<", "&"];
function text(node, _2, parent, state) {
  return parent && parent.type === "element" && (parent.tagName === "script" || parent.tagName === "style") ? node.value : stringifyEntities(
    node.value,
    Object.assign({}, state.settings.characterReferences, {
      subset: textEntitySubset
    })
  );
}
function raw(node, index, parent, state) {
  return state.settings.allowDangerousHtml ? node.value : text(node, index, parent, state);
}
function root(node, _1, _2, state) {
  return state.all(node);
}
const handle = zwitch("type", {
  invalid,
  unknown,
  handlers: { comment, doctype, element, raw, root, text }
});
function invalid(node) {
  throw new Error("Expected node, not `" + node + "`");
}
function unknown(node_) {
  const node = (
    /** @type {Nodes} */
    node_
  );
  throw new Error("Cannot compile unknown node `" + node.type + "`");
}
const emptyOptions = {};
const emptyCharacterReferences = {};
const emptyChildren = [];
function toHtml(tree, options) {
  const options_ = options || emptyOptions;
  const quote = options_.quote || '"';
  const alternative = quote === '"' ? "'" : '"';
  if (quote !== '"' && quote !== "'") {
    throw new Error("Invalid quote `" + quote + "`, expected `'` or `\"`");
  }
  const state = {
    one,
    all,
    settings: {
      omitOptionalTags: options_.omitOptionalTags || false,
      allowParseErrors: options_.allowParseErrors || false,
      allowDangerousCharacters: options_.allowDangerousCharacters || false,
      quoteSmart: options_.quoteSmart || false,
      preferUnquoted: options_.preferUnquoted || false,
      tightAttributes: options_.tightAttributes || false,
      upperDoctype: options_.upperDoctype || false,
      tightDoctype: options_.tightDoctype || false,
      bogusComments: options_.bogusComments || false,
      tightCommaSeparatedLists: options_.tightCommaSeparatedLists || false,
      tightSelfClosing: options_.tightSelfClosing || false,
      collapseEmptyAttributes: options_.collapseEmptyAttributes || false,
      allowDangerousHtml: options_.allowDangerousHtml || false,
      voids: options_.voids || htmlVoidElements,
      characterReferences: options_.characterReferences || emptyCharacterReferences,
      closeSelfClosing: options_.closeSelfClosing || false,
      closeEmptyElements: options_.closeEmptyElements || false
    },
    schema: options_.space === "svg" ? svg : html$2,
    quote,
    alternative
  };
  return state.one(
    Array.isArray(tree) ? { type: "root", children: tree } : tree,
    void 0,
    void 0
  );
}
function one(node, index, parent) {
  return handle(node, index, parent, this);
}
function all(parent) {
  const results = [];
  const children = parent && parent.children || emptyChildren;
  let index = -1;
  while (++index < children.length) {
    results[index] = this.one(children[index], index, parent);
  }
  return results.join("");
}
function resolveColorReplacements(theme, options) {
  const replacements = typeof theme === "string" ? {} : { ...theme.colorReplacements };
  const themeName = typeof theme === "string" ? theme : theme.name;
  for (const [key2, value] of Object.entries((options == null ? void 0 : options.colorReplacements) || {})) {
    if (typeof value === "string")
      replacements[key2] = value;
    else if (key2 === themeName)
      Object.assign(replacements, value);
  }
  return replacements;
}
function applyColorReplacements(color, replacements) {
  if (!color)
    return color;
  return (replacements == null ? void 0 : replacements[color == null ? void 0 : color.toLowerCase()]) || color;
}
function toArray(x2) {
  return Array.isArray(x2) ? x2 : [x2];
}
async function normalizeGetter(p2) {
  return Promise.resolve(typeof p2 === "function" ? p2() : p2).then((r2) => r2.default || r2);
}
function isPlainLang(lang) {
  return !lang || ["plaintext", "txt", "text", "plain"].includes(lang);
}
function isSpecialLang(lang) {
  return lang === "ansi" || isPlainLang(lang);
}
function isNoneTheme(theme) {
  return theme === "none";
}
function isSpecialTheme(theme) {
  return isNoneTheme(theme);
}
function addClassToHast(node, className) {
  var _a2;
  if (!className)
    return node;
  node.properties || (node.properties = {});
  (_a2 = node.properties).class || (_a2.class = []);
  if (typeof node.properties.class === "string")
    node.properties.class = node.properties.class.split(/\s+/g);
  if (!Array.isArray(node.properties.class))
    node.properties.class = [];
  const targets = Array.isArray(className) ? className : className.split(/\s+/g);
  for (const c of targets) {
    if (c && !node.properties.class.includes(c))
      node.properties.class.push(c);
  }
  return node;
}
function splitLines(code, preserveEnding = false) {
  var _a2;
  if (code.length === 0) {
    return [["", 0]];
  }
  const parts = code.split(/(\r?\n)/g);
  let index = 0;
  const lines = [];
  for (let i2 = 0; i2 < parts.length; i2 += 2) {
    const line = preserveEnding ? parts[i2] + (parts[i2 + 1] || "") : parts[i2];
    lines.push([line, index]);
    index += parts[i2].length;
    index += ((_a2 = parts[i2 + 1]) == null ? void 0 : _a2.length) || 0;
  }
  return lines;
}
function createPositionConverter(code) {
  const lines = splitLines(code, true).map(([line]) => line);
  function indexToPos(index) {
    if (index === code.length) {
      return {
        line: lines.length - 1,
        character: lines[lines.length - 1].length
      };
    }
    let character = index;
    let line = 0;
    for (const lineText of lines) {
      if (character < lineText.length)
        break;
      character -= lineText.length;
      line++;
    }
    return { line, character };
  }
  function posToIndex(line, character) {
    let index = 0;
    for (let i2 = 0; i2 < line; i2++)
      index += lines[i2].length;
    index += character;
    return index;
  }
  return {
    lines,
    indexToPos,
    posToIndex
  };
}
function guessEmbeddedLanguages(code, _lang, highlighter) {
  const langs = /* @__PURE__ */ new Set();
  for (const match of code.matchAll(/:?lang=["']([^"']+)["']/g)) {
    const lang = match[1].toLowerCase().trim();
    if (lang)
      langs.add(lang);
  }
  for (const match of code.matchAll(/(?:```|~~~)([\w-]+)/g)) {
    const lang = match[1].toLowerCase().trim();
    if (lang)
      langs.add(lang);
  }
  for (const match of code.matchAll(/\\begin\{([\w-]+)\}/g)) {
    const lang = match[1].toLowerCase().trim();
    if (lang)
      langs.add(lang);
  }
  for (const match of code.matchAll(/<script\s+(?:type|lang)=["']([^"']+)["']/gi)) {
    const fullType = match[1].toLowerCase().trim();
    const lang = fullType.includes("/") ? fullType.split("/").pop() : fullType;
    if (lang)
      langs.add(lang);
  }
  if (!highlighter)
    return Array.from(langs);
  const bundle = highlighter.getBundledLanguages();
  return Array.from(langs).filter((l2) => l2 && bundle[l2]);
}
const DEFAULT_COLOR_LIGHT_DARK = "light-dark()";
const COLOR_KEYS = ["color", "background-color"];
function splitToken(token2, offsets) {
  let lastOffset = 0;
  const tokens = [];
  for (const offset of offsets) {
    if (offset > lastOffset) {
      tokens.push({
        ...token2,
        content: token2.content.slice(lastOffset, offset),
        offset: token2.offset + lastOffset
      });
    }
    lastOffset = offset;
  }
  if (lastOffset < token2.content.length) {
    tokens.push({
      ...token2,
      content: token2.content.slice(lastOffset),
      offset: token2.offset + lastOffset
    });
  }
  return tokens;
}
function splitTokens(tokens, breakpoints) {
  const sorted = Array.from(breakpoints instanceof Set ? breakpoints : new Set(breakpoints)).sort((a, b2) => a - b2);
  if (!sorted.length)
    return tokens;
  return tokens.map((line) => {
    return line.flatMap((token2) => {
      const breakpointsInToken = sorted.filter((i2) => token2.offset < i2 && i2 < token2.offset + token2.content.length).map((i2) => i2 - token2.offset).sort((a, b2) => a - b2);
      if (!breakpointsInToken.length)
        return token2;
      return splitToken(token2, breakpointsInToken);
    });
  });
}
function flatTokenVariants(merged, variantsOrder, cssVariablePrefix, defaultColor, colorsRendering = "css-vars") {
  const token2 = {
    content: merged.content,
    explanation: merged.explanation,
    offset: merged.offset
  };
  const styles = variantsOrder.map((t) => getTokenStyleObject(merged.variants[t]));
  const styleKeys = new Set(styles.flatMap((t) => Object.keys(t)));
  const mergedStyles = {};
  const varKey = (idx, key2) => {
    const keyName = key2 === "color" ? "" : key2 === "background-color" ? "-bg" : `-${key2}`;
    return cssVariablePrefix + variantsOrder[idx] + (key2 === "color" ? "" : keyName);
  };
  styles.forEach((cur, idx) => {
    for (const key2 of styleKeys) {
      const value = cur[key2] || "inherit";
      if (idx === 0 && defaultColor && COLOR_KEYS.includes(key2)) {
        if (defaultColor === DEFAULT_COLOR_LIGHT_DARK && styles.length > 1) {
          const lightIndex = variantsOrder.findIndex((t) => t === "light");
          const darkIndex = variantsOrder.findIndex((t) => t === "dark");
          if (lightIndex === -1 || darkIndex === -1)
            throw new ShikiError$2('When using `defaultColor: "light-dark()"`, you must provide both `light` and `dark` themes');
          const lightValue = styles[lightIndex][key2] || "inherit";
          const darkValue = styles[darkIndex][key2] || "inherit";
          mergedStyles[key2] = `light-dark(${lightValue}, ${darkValue})`;
          if (colorsRendering === "css-vars")
            mergedStyles[varKey(idx, key2)] = value;
        } else {
          mergedStyles[key2] = value;
        }
      } else {
        if (colorsRendering === "css-vars")
          mergedStyles[varKey(idx, key2)] = value;
      }
    }
  });
  token2.htmlStyle = mergedStyles;
  return token2;
}
function getTokenStyleObject(token2) {
  const styles = {};
  if (token2.color)
    styles.color = token2.color;
  if (token2.bgColor)
    styles["background-color"] = token2.bgColor;
  if (token2.fontStyle) {
    if (token2.fontStyle & FontStyle.Italic)
      styles["font-style"] = "italic";
    if (token2.fontStyle & FontStyle.Bold)
      styles["font-weight"] = "bold";
    const decorations2 = [];
    if (token2.fontStyle & FontStyle.Underline)
      decorations2.push("underline");
    if (token2.fontStyle & FontStyle.Strikethrough)
      decorations2.push("line-through");
    if (decorations2.length)
      styles["text-decoration"] = decorations2.join(" ");
  }
  return styles;
}
function stringifyTokenStyle(token2) {
  if (typeof token2 === "string")
    return token2;
  return Object.entries(token2).map(([key2, value]) => `${key2}:${value}`).join(";");
}
const _grammarStateMap = /* @__PURE__ */ new WeakMap();
function setLastGrammarStateToMap(keys, state) {
  _grammarStateMap.set(keys, state);
}
function getLastGrammarStateFromMap(keys) {
  return _grammarStateMap.get(keys);
}
class GrammarState {
  constructor(...args) {
    /**
     * Theme to Stack mapping
     */
    __publicField(this, "_stacks", {});
    __publicField(this, "lang");
    if (args.length === 2) {
      const [stacksMap, lang] = args;
      this.lang = lang;
      this._stacks = stacksMap;
    } else {
      const [stack, lang, theme] = args;
      this.lang = lang;
      this._stacks = { [theme]: stack };
    }
  }
  get themes() {
    return Object.keys(this._stacks);
  }
  get theme() {
    return this.themes[0];
  }
  get _stack() {
    return this._stacks[this.theme];
  }
  /**
   * Static method to create a initial grammar state.
   */
  static initial(lang, themes) {
    return new GrammarState(
      Object.fromEntries(toArray(themes).map((theme) => [theme, INITIAL])),
      lang
    );
  }
  /**
   * Get the internal stack object.
   * @internal
   */
  getInternalStack(theme = this.theme) {
    return this._stacks[theme];
  }
  getScopes(theme = this.theme) {
    return getScopes(this._stacks[theme]);
  }
  toJSON() {
    return {
      lang: this.lang,
      theme: this.theme,
      themes: this.themes,
      scopes: this.getScopes()
    };
  }
}
function getScopes(stack) {
  const scopes = [];
  const visited = /* @__PURE__ */ new Set();
  function pushScope(stack2) {
    var _a2;
    if (visited.has(stack2))
      return;
    visited.add(stack2);
    const name = (_a2 = stack2 == null ? void 0 : stack2.nameScopesList) == null ? void 0 : _a2.scopeName;
    if (name)
      scopes.push(name);
    if (stack2.parent)
      pushScope(stack2.parent);
  }
  pushScope(stack);
  return scopes;
}
function getGrammarStack(state, theme) {
  if (!(state instanceof GrammarState))
    throw new ShikiError$2("Invalid grammar state");
  return state.getInternalStack(theme);
}
function transformerDecorations() {
  const map = /* @__PURE__ */ new WeakMap();
  function getContext(shiki) {
    if (!map.has(shiki.meta)) {
      let normalizePosition = function(p2) {
        if (typeof p2 === "number") {
          if (p2 < 0 || p2 > shiki.source.length)
            throw new ShikiError$2(`Invalid decoration offset: ${p2}. Code length: ${shiki.source.length}`);
          return {
            ...converter.indexToPos(p2),
            offset: p2
          };
        } else {
          const line = converter.lines[p2.line];
          if (line === void 0)
            throw new ShikiError$2(`Invalid decoration position ${JSON.stringify(p2)}. Lines length: ${converter.lines.length}`);
          let character = p2.character;
          if (character < 0)
            character = line.length + character;
          if (character < 0 || character > line.length)
            throw new ShikiError$2(`Invalid decoration position ${JSON.stringify(p2)}. Line ${p2.line} length: ${line.length}`);
          return {
            ...p2,
            character,
            offset: converter.posToIndex(p2.line, character)
          };
        }
      };
      const converter = createPositionConverter(shiki.source);
      const decorations2 = (shiki.options.decorations || []).map((d2) => ({
        ...d2,
        start: normalizePosition(d2.start),
        end: normalizePosition(d2.end)
      }));
      verifyIntersections(decorations2);
      map.set(shiki.meta, {
        decorations: decorations2,
        converter,
        source: shiki.source
      });
    }
    return map.get(shiki.meta);
  }
  return {
    name: "shiki:decorations",
    tokens(tokens) {
      var _a2;
      if (!((_a2 = this.options.decorations) == null ? void 0 : _a2.length))
        return;
      const ctx = getContext(this);
      const breakpoints = ctx.decorations.flatMap((d2) => [d2.start.offset, d2.end.offset]);
      const splitted = splitTokens(tokens, breakpoints);
      return splitted;
    },
    code(codeEl) {
      var _a2;
      if (!((_a2 = this.options.decorations) == null ? void 0 : _a2.length))
        return;
      const ctx = getContext(this);
      const lines = Array.from(codeEl.children).filter((i2) => i2.type === "element" && i2.tagName === "span");
      if (lines.length !== ctx.converter.lines.length)
        throw new ShikiError$2(`Number of lines in code element (${lines.length}) does not match the number of lines in the source (${ctx.converter.lines.length}). Failed to apply decorations.`);
      function applyLineSection(line, start, end, decoration) {
        const lineEl = lines[line];
        let text2 = "";
        let startIndex = -1;
        let endIndex = -1;
        if (start === 0)
          startIndex = 0;
        if (end === 0)
          endIndex = 0;
        if (end === Number.POSITIVE_INFINITY)
          endIndex = lineEl.children.length;
        if (startIndex === -1 || endIndex === -1) {
          for (let i2 = 0; i2 < lineEl.children.length; i2++) {
            text2 += stringify(lineEl.children[i2]);
            if (startIndex === -1 && text2.length === start)
              startIndex = i2 + 1;
            if (endIndex === -1 && text2.length === end)
              endIndex = i2 + 1;
          }
        }
        if (startIndex === -1)
          throw new ShikiError$2(`Failed to find start index for decoration ${JSON.stringify(decoration.start)}`);
        if (endIndex === -1)
          throw new ShikiError$2(`Failed to find end index for decoration ${JSON.stringify(decoration.end)}`);
        const children = lineEl.children.slice(startIndex, endIndex);
        if (!decoration.alwaysWrap && children.length === lineEl.children.length) {
          applyDecoration(lineEl, decoration, "line");
        } else if (!decoration.alwaysWrap && children.length === 1 && children[0].type === "element") {
          applyDecoration(children[0], decoration, "token");
        } else {
          const wrapper = {
            type: "element",
            tagName: "span",
            properties: {},
            children
          };
          applyDecoration(wrapper, decoration, "wrapper");
          lineEl.children.splice(startIndex, children.length, wrapper);
        }
      }
      function applyLine(line, decoration) {
        lines[line] = applyDecoration(lines[line], decoration, "line");
      }
      function applyDecoration(el, decoration, type) {
        var _a3;
        const properties = decoration.properties || {};
        const transform2 = decoration.transform || ((i2) => i2);
        el.tagName = decoration.tagName || "span";
        el.properties = {
          ...el.properties,
          ...properties,
          class: el.properties.class
        };
        if ((_a3 = decoration.properties) == null ? void 0 : _a3.class)
          addClassToHast(el, decoration.properties.class);
        el = transform2(el, type) || el;
        return el;
      }
      const lineApplies = [];
      const sorted = ctx.decorations.sort((a, b2) => b2.start.offset - a.start.offset || a.end.offset - b2.end.offset);
      for (const decoration of sorted) {
        const { start, end } = decoration;
        if (start.line === end.line) {
          applyLineSection(start.line, start.character, end.character, decoration);
        } else if (start.line < end.line) {
          applyLineSection(start.line, start.character, Number.POSITIVE_INFINITY, decoration);
          for (let i2 = start.line + 1; i2 < end.line; i2++)
            lineApplies.unshift(() => applyLine(i2, decoration));
          applyLineSection(end.line, 0, end.character, decoration);
        }
      }
      lineApplies.forEach((i2) => i2());
    }
  };
}
function verifyIntersections(items) {
  for (let i2 = 0; i2 < items.length; i2++) {
    const foo = items[i2];
    if (foo.start.offset > foo.end.offset)
      throw new ShikiError$2(`Invalid decoration range: ${JSON.stringify(foo.start)} - ${JSON.stringify(foo.end)}`);
    for (let j2 = i2 + 1; j2 < items.length; j2++) {
      const bar = items[j2];
      const isFooHasBarStart = foo.start.offset <= bar.start.offset && bar.start.offset < foo.end.offset;
      const isFooHasBarEnd = foo.start.offset < bar.end.offset && bar.end.offset <= foo.end.offset;
      const isBarHasFooStart = bar.start.offset <= foo.start.offset && foo.start.offset < bar.end.offset;
      const isBarHasFooEnd = bar.start.offset < foo.end.offset && foo.end.offset <= bar.end.offset;
      if (isFooHasBarStart || isFooHasBarEnd || isBarHasFooStart || isBarHasFooEnd) {
        if (isFooHasBarStart && isFooHasBarEnd)
          continue;
        if (isBarHasFooStart && isBarHasFooEnd)
          continue;
        if (isBarHasFooStart && foo.start.offset === foo.end.offset)
          continue;
        if (isFooHasBarEnd && bar.start.offset === bar.end.offset)
          continue;
        throw new ShikiError$2(`Decorations ${JSON.stringify(foo.start)} and ${JSON.stringify(bar.start)} intersect.`);
      }
    }
  }
}
function stringify(el) {
  if (el.type === "text")
    return el.value;
  if (el.type === "element")
    return el.children.map(stringify).join("");
  return "";
}
const builtInTransformers = [
  /* @__PURE__ */ transformerDecorations()
];
function getTransformers(options) {
  const transformers = sortTransformersByEnforcement(options.transformers || []);
  return [
    ...transformers.pre,
    ...transformers.normal,
    ...transformers.post,
    ...builtInTransformers
  ];
}
function sortTransformersByEnforcement(transformers) {
  const pre = [];
  const post = [];
  const normal = [];
  for (const transformer of transformers) {
    switch (transformer.enforce) {
      case "pre":
        pre.push(transformer);
        break;
      case "post":
        post.push(transformer);
        break;
      default:
        normal.push(transformer);
    }
  }
  return { pre, post, normal };
}
var namedColors = [
  "black",
  "red",
  "green",
  "yellow",
  "blue",
  "magenta",
  "cyan",
  "white",
  "brightBlack",
  "brightRed",
  "brightGreen",
  "brightYellow",
  "brightBlue",
  "brightMagenta",
  "brightCyan",
  "brightWhite"
];
var decorations = {
  1: "bold",
  2: "dim",
  3: "italic",
  4: "underline",
  7: "reverse",
  8: "hidden",
  9: "strikethrough"
};
function findSequence(value, position) {
  const nextEscape = value.indexOf("\x1B", position);
  if (nextEscape !== -1) {
    if (value[nextEscape + 1] === "[") {
      const nextClose = value.indexOf("m", nextEscape);
      if (nextClose !== -1) {
        return {
          sequence: value.substring(nextEscape + 2, nextClose).split(";"),
          startPosition: nextEscape,
          position: nextClose + 1
        };
      }
    }
  }
  return {
    position: value.length
  };
}
function parseColor(sequence) {
  const colorMode = sequence.shift();
  if (colorMode === "2") {
    const rgb = sequence.splice(0, 3).map((x2) => Number.parseInt(x2));
    if (rgb.length !== 3 || rgb.some((x2) => Number.isNaN(x2)))
      return;
    return {
      type: "rgb",
      rgb
    };
  } else if (colorMode === "5") {
    const index = sequence.shift();
    if (index) {
      return { type: "table", index: Number(index) };
    }
  }
}
function parseSequence(sequence) {
  const commands = [];
  while (sequence.length > 0) {
    const code = sequence.shift();
    if (!code)
      continue;
    const codeInt = Number.parseInt(code);
    if (Number.isNaN(codeInt))
      continue;
    if (codeInt === 0) {
      commands.push({ type: "resetAll" });
    } else if (codeInt <= 9) {
      const decoration = decorations[codeInt];
      if (decoration) {
        commands.push({
          type: "setDecoration",
          value: decorations[codeInt]
        });
      }
    } else if (codeInt <= 29) {
      const decoration = decorations[codeInt - 20];
      if (decoration) {
        commands.push({
          type: "resetDecoration",
          value: decoration
        });
        if (decoration === "dim") {
          commands.push({
            type: "resetDecoration",
            value: "bold"
          });
        }
      }
    } else if (codeInt <= 37) {
      commands.push({
        type: "setForegroundColor",
        value: { type: "named", name: namedColors[codeInt - 30] }
      });
    } else if (codeInt === 38) {
      const color = parseColor(sequence);
      if (color) {
        commands.push({
          type: "setForegroundColor",
          value: color
        });
      }
    } else if (codeInt === 39) {
      commands.push({
        type: "resetForegroundColor"
      });
    } else if (codeInt <= 47) {
      commands.push({
        type: "setBackgroundColor",
        value: { type: "named", name: namedColors[codeInt - 40] }
      });
    } else if (codeInt === 48) {
      const color = parseColor(sequence);
      if (color) {
        commands.push({
          type: "setBackgroundColor",
          value: color
        });
      }
    } else if (codeInt === 49) {
      commands.push({
        type: "resetBackgroundColor"
      });
    } else if (codeInt === 53) {
      commands.push({
        type: "setDecoration",
        value: "overline"
      });
    } else if (codeInt === 55) {
      commands.push({
        type: "resetDecoration",
        value: "overline"
      });
    } else if (codeInt >= 90 && codeInt <= 97) {
      commands.push({
        type: "setForegroundColor",
        value: { type: "named", name: namedColors[codeInt - 90 + 8] }
      });
    } else if (codeInt >= 100 && codeInt <= 107) {
      commands.push({
        type: "setBackgroundColor",
        value: { type: "named", name: namedColors[codeInt - 100 + 8] }
      });
    }
  }
  return commands;
}
function createAnsiSequenceParser() {
  let foreground = null;
  let background = null;
  let decorations2 = /* @__PURE__ */ new Set();
  return {
    parse(value) {
      const tokens = [];
      let position = 0;
      do {
        const findResult = findSequence(value, position);
        const text2 = findResult.sequence ? value.substring(position, findResult.startPosition) : value.substring(position);
        if (text2.length > 0) {
          tokens.push({
            value: text2,
            foreground,
            background,
            decorations: new Set(decorations2)
          });
        }
        if (findResult.sequence) {
          const commands = parseSequence(findResult.sequence);
          for (const styleToken of commands) {
            if (styleToken.type === "resetAll") {
              foreground = null;
              background = null;
              decorations2.clear();
            } else if (styleToken.type === "resetForegroundColor") {
              foreground = null;
            } else if (styleToken.type === "resetBackgroundColor") {
              background = null;
            } else if (styleToken.type === "resetDecoration") {
              decorations2.delete(styleToken.value);
            }
          }
          for (const styleToken of commands) {
            if (styleToken.type === "setForegroundColor") {
              foreground = styleToken.value;
            } else if (styleToken.type === "setBackgroundColor") {
              background = styleToken.value;
            } else if (styleToken.type === "setDecoration") {
              decorations2.add(styleToken.value);
            }
          }
        }
        position = findResult.position;
      } while (position < value.length);
      return tokens;
    }
  };
}
var defaultNamedColorsMap = {
  black: "#000000",
  red: "#bb0000",
  green: "#00bb00",
  yellow: "#bbbb00",
  blue: "#0000bb",
  magenta: "#ff00ff",
  cyan: "#00bbbb",
  white: "#eeeeee",
  brightBlack: "#555555",
  brightRed: "#ff5555",
  brightGreen: "#00ff00",
  brightYellow: "#ffff55",
  brightBlue: "#5555ff",
  brightMagenta: "#ff55ff",
  brightCyan: "#55ffff",
  brightWhite: "#ffffff"
};
function createColorPalette(namedColorsMap = defaultNamedColorsMap) {
  function namedColor(name) {
    return namedColorsMap[name];
  }
  function rgbColor(rgb) {
    return `#${rgb.map((x2) => Math.max(0, Math.min(x2, 255)).toString(16).padStart(2, "0")).join("")}`;
  }
  let colorTable;
  function getColorTable() {
    if (colorTable) {
      return colorTable;
    }
    colorTable = [];
    for (let i2 = 0; i2 < namedColors.length; i2++) {
      colorTable.push(namedColor(namedColors[i2]));
    }
    let levels = [0, 95, 135, 175, 215, 255];
    for (let r2 = 0; r2 < 6; r2++) {
      for (let g = 0; g < 6; g++) {
        for (let b2 = 0; b2 < 6; b2++) {
          colorTable.push(rgbColor([levels[r2], levels[g], levels[b2]]));
        }
      }
    }
    let level = 8;
    for (let i2 = 0; i2 < 24; i2++, level += 10) {
      colorTable.push(rgbColor([level, level, level]));
    }
    return colorTable;
  }
  function tableColor(index) {
    return getColorTable()[index];
  }
  function value(color) {
    switch (color.type) {
      case "named":
        return namedColor(color.name);
      case "rgb":
        return rgbColor(color.rgb);
      case "table":
        return tableColor(color.index);
    }
  }
  return {
    value
  };
}
const defaultAnsiColors = {
  black: "#000000",
  red: "#cd3131",
  green: "#0DBC79",
  yellow: "#E5E510",
  blue: "#2472C8",
  magenta: "#BC3FBC",
  cyan: "#11A8CD",
  white: "#E5E5E5",
  brightBlack: "#666666",
  brightRed: "#F14C4C",
  brightGreen: "#23D18B",
  brightYellow: "#F5F543",
  brightBlue: "#3B8EEA",
  brightMagenta: "#D670D6",
  brightCyan: "#29B8DB",
  brightWhite: "#FFFFFF"
};
function tokenizeAnsiWithTheme(theme, fileContents, options) {
  const colorReplacements = resolveColorReplacements(theme, options);
  const lines = splitLines(fileContents);
  const ansiPalette = Object.fromEntries(
    namedColors.map((name) => {
      var _a2;
      const key2 = `terminal.ansi${name[0].toUpperCase()}${name.substring(1)}`;
      const themeColor = (_a2 = theme.colors) == null ? void 0 : _a2[key2];
      return [name, themeColor || defaultAnsiColors[name]];
    })
  );
  const colorPalette = createColorPalette(ansiPalette);
  const parser = createAnsiSequenceParser();
  return lines.map(
    (line) => parser.parse(line[0]).map((token2) => {
      let color;
      let bgColor;
      if (token2.decorations.has("reverse")) {
        color = token2.background ? colorPalette.value(token2.background) : theme.bg;
        bgColor = token2.foreground ? colorPalette.value(token2.foreground) : theme.fg;
      } else {
        color = token2.foreground ? colorPalette.value(token2.foreground) : theme.fg;
        bgColor = token2.background ? colorPalette.value(token2.background) : void 0;
      }
      color = applyColorReplacements(color, colorReplacements);
      bgColor = applyColorReplacements(bgColor, colorReplacements);
      if (token2.decorations.has("dim"))
        color = dimColor(color);
      let fontStyle = FontStyle.None;
      if (token2.decorations.has("bold"))
        fontStyle |= FontStyle.Bold;
      if (token2.decorations.has("italic"))
        fontStyle |= FontStyle.Italic;
      if (token2.decorations.has("underline"))
        fontStyle |= FontStyle.Underline;
      if (token2.decorations.has("strikethrough"))
        fontStyle |= FontStyle.Strikethrough;
      return {
        content: token2.value,
        offset: line[1],
        // TODO: more accurate offset? might need to fork ansi-sequence-parser
        color,
        bgColor,
        fontStyle
      };
    })
  );
}
function dimColor(color) {
  const hexMatch = color.match(/#([0-9a-f]{3,8})/i);
  if (hexMatch) {
    const hex = hexMatch[1];
    if (hex.length === 8) {
      const alpha = Math.round(Number.parseInt(hex.slice(6, 8), 16) / 2).toString(16).padStart(2, "0");
      return `#${hex.slice(0, 6)}${alpha}`;
    } else if (hex.length === 6) {
      return `#${hex}80`;
    } else if (hex.length === 4) {
      const r2 = hex[0];
      const g = hex[1];
      const b2 = hex[2];
      const a = hex[3];
      const alpha = Math.round(Number.parseInt(`${a}${a}`, 16) / 2).toString(16).padStart(2, "0");
      return `#${r2}${r2}${g}${g}${b2}${b2}${alpha}`;
    } else if (hex.length === 3) {
      const r2 = hex[0];
      const g = hex[1];
      const b2 = hex[2];
      return `#${r2}${r2}${g}${g}${b2}${b2}80`;
    }
  }
  const cssVarMatch = color.match(/var\((--[\w-]+-ansi-[\w-]+)\)/);
  if (cssVarMatch)
    return `var(${cssVarMatch[1]}-dim)`;
  return color;
}
function codeToTokensBase$1(internal, code, options = {}) {
  const {
    theme: themeName = internal.getLoadedThemes()[0]
  } = options;
  const lang = internal.resolveLangAlias(options.lang || "text");
  if (isPlainLang(lang) || isNoneTheme(themeName))
    return splitLines(code).map((line) => [{ content: line[0], offset: line[1] }]);
  const { theme, colorMap } = internal.setTheme(themeName);
  if (lang === "ansi")
    return tokenizeAnsiWithTheme(theme, code, options);
  const _grammar = internal.getLanguage(options.lang || "text");
  if (options.grammarState) {
    if (options.grammarState.lang !== _grammar.name) {
      throw new ShikiError$2(`Grammar state language "${options.grammarState.lang}" does not match highlight language "${_grammar.name}"`);
    }
    if (!options.grammarState.themes.includes(theme.name)) {
      throw new ShikiError$2(`Grammar state themes "${options.grammarState.themes}" do not contain highlight theme "${theme.name}"`);
    }
  }
  return tokenizeWithTheme(code, _grammar, theme, colorMap, options);
}
function getLastGrammarState$1(...args) {
  if (args.length === 2) {
    return getLastGrammarStateFromMap(args[1]);
  }
  const [internal, code, options = {}] = args;
  const {
    lang = "text",
    theme: themeName = internal.getLoadedThemes()[0]
  } = options;
  if (isPlainLang(lang) || isNoneTheme(themeName))
    throw new ShikiError$2("Plain language does not have grammar state");
  if (lang === "ansi")
    throw new ShikiError$2("ANSI language does not have grammar state");
  const { theme, colorMap } = internal.setTheme(themeName);
  const _grammar = internal.getLanguage(lang);
  return new GrammarState(
    _tokenizeWithTheme(code, _grammar, theme, colorMap, options).stateStack,
    _grammar.name,
    theme.name
  );
}
function tokenizeWithTheme(code, grammar, theme, colorMap, options) {
  const result = _tokenizeWithTheme(code, grammar, theme, colorMap, options);
  const grammarState = new GrammarState(
    result.stateStack,
    grammar.name,
    theme.name
  );
  setLastGrammarStateToMap(result.tokens, grammarState);
  return result.tokens;
}
function _tokenizeWithTheme(code, grammar, theme, colorMap, options) {
  const colorReplacements = resolveColorReplacements(theme, options);
  const {
    tokenizeMaxLineLength = 0,
    tokenizeTimeLimit = 500
  } = options;
  const lines = splitLines(code);
  let stateStack = options.grammarState ? getGrammarStack(options.grammarState, theme.name) ?? INITIAL : options.grammarContextCode != null ? _tokenizeWithTheme(
    options.grammarContextCode,
    grammar,
    theme,
    colorMap,
    {
      ...options,
      grammarState: void 0,
      grammarContextCode: void 0
    }
  ).stateStack : INITIAL;
  let actual = [];
  const final = [];
  for (let i2 = 0, len = lines.length; i2 < len; i2++) {
    const [line, lineOffset] = lines[i2];
    if (line === "") {
      actual = [];
      final.push([]);
      continue;
    }
    if (tokenizeMaxLineLength > 0 && line.length >= tokenizeMaxLineLength) {
      actual = [];
      final.push([{
        content: line,
        offset: lineOffset,
        color: "",
        fontStyle: 0
      }]);
      continue;
    }
    let resultWithScopes;
    let tokensWithScopes;
    let tokensWithScopesIndex;
    if (options.includeExplanation) {
      resultWithScopes = grammar.tokenizeLine(line, stateStack, tokenizeTimeLimit);
      tokensWithScopes = resultWithScopes.tokens;
      tokensWithScopesIndex = 0;
    }
    const result = grammar.tokenizeLine2(line, stateStack, tokenizeTimeLimit);
    const tokensLength = result.tokens.length / 2;
    for (let j2 = 0; j2 < tokensLength; j2++) {
      const startIndex = result.tokens[2 * j2];
      const nextStartIndex = j2 + 1 < tokensLength ? result.tokens[2 * j2 + 2] : line.length;
      if (startIndex === nextStartIndex)
        continue;
      const metadata = result.tokens[2 * j2 + 1];
      const color = applyColorReplacements(
        colorMap[EncodedTokenMetadata.getForeground(metadata)],
        colorReplacements
      );
      const fontStyle = EncodedTokenMetadata.getFontStyle(metadata);
      const token2 = {
        content: line.substring(startIndex, nextStartIndex),
        offset: lineOffset + startIndex,
        color,
        fontStyle
      };
      if (options.includeExplanation) {
        const themeSettingsSelectors = [];
        if (options.includeExplanation !== "scopeName") {
          for (const setting of theme.settings) {
            let selectors;
            switch (typeof setting.scope) {
              case "string":
                selectors = setting.scope.split(/,/).map((scope) => scope.trim());
                break;
              case "object":
                selectors = setting.scope;
                break;
              default:
                continue;
            }
            themeSettingsSelectors.push({
              settings: setting,
              selectors: selectors.map((selector) => selector.split(/ /))
            });
          }
        }
        token2.explanation = [];
        let offset = 0;
        while (startIndex + offset < nextStartIndex) {
          const tokenWithScopes = tokensWithScopes[tokensWithScopesIndex];
          const tokenWithScopesText = line.substring(
            tokenWithScopes.startIndex,
            tokenWithScopes.endIndex
          );
          offset += tokenWithScopesText.length;
          token2.explanation.push({
            content: tokenWithScopesText,
            scopes: options.includeExplanation === "scopeName" ? explainThemeScopesNameOnly(
              tokenWithScopes.scopes
            ) : explainThemeScopesFull(
              themeSettingsSelectors,
              tokenWithScopes.scopes
            )
          });
          tokensWithScopesIndex += 1;
        }
      }
      actual.push(token2);
    }
    final.push(actual);
    actual = [];
    stateStack = result.ruleStack;
  }
  return {
    tokens: final,
    stateStack
  };
}
function explainThemeScopesNameOnly(scopes) {
  return scopes.map((scope) => ({ scopeName: scope }));
}
function explainThemeScopesFull(themeSelectors, scopes) {
  const result = [];
  for (let i2 = 0, len = scopes.length; i2 < len; i2++) {
    const scope = scopes[i2];
    result[i2] = {
      scopeName: scope,
      themeMatches: explainThemeScope(themeSelectors, scope, scopes.slice(0, i2))
    };
  }
  return result;
}
function matchesOne(selector, scope) {
  return selector === scope || scope.substring(0, selector.length) === selector && scope[selector.length] === ".";
}
function matches(selectors, scope, parentScopes) {
  if (!matchesOne(selectors[selectors.length - 1], scope))
    return false;
  let selectorParentIndex = selectors.length - 2;
  let parentIndex = parentScopes.length - 1;
  while (selectorParentIndex >= 0 && parentIndex >= 0) {
    if (matchesOne(selectors[selectorParentIndex], parentScopes[parentIndex]))
      selectorParentIndex -= 1;
    parentIndex -= 1;
  }
  if (selectorParentIndex === -1)
    return true;
  return false;
}
function explainThemeScope(themeSettingsSelectors, scope, parentScopes) {
  const result = [];
  for (const { selectors, settings } of themeSettingsSelectors) {
    for (const selectorPieces of selectors) {
      if (matches(selectorPieces, scope, parentScopes)) {
        result.push(settings);
        break;
      }
    }
  }
  return result;
}
function codeToTokensWithThemes$1(internal, code, options) {
  const themes = Object.entries(options.themes).filter((i2) => i2[1]).map((i2) => ({ color: i2[0], theme: i2[1] }));
  const themedTokens = themes.map((t) => {
    const tokens2 = codeToTokensBase$1(internal, code, {
      ...options,
      theme: t.theme
    });
    const state = getLastGrammarStateFromMap(tokens2);
    const theme = typeof t.theme === "string" ? t.theme : t.theme.name;
    return {
      tokens: tokens2,
      state,
      theme
    };
  });
  const tokens = syncThemesTokenization(
    ...themedTokens.map((i2) => i2.tokens)
  );
  const mergedTokens = tokens[0].map(
    (line, lineIdx) => line.map((_token, tokenIdx) => {
      const mergedToken = {
        content: _token.content,
        variants: {},
        offset: _token.offset
      };
      if ("includeExplanation" in options && options.includeExplanation) {
        mergedToken.explanation = _token.explanation;
      }
      tokens.forEach((t, themeIdx) => {
        const {
          content: _2,
          explanation: __,
          offset: ___,
          ...styles
        } = t[lineIdx][tokenIdx];
        mergedToken.variants[themes[themeIdx].color] = styles;
      });
      return mergedToken;
    })
  );
  const mergedGrammarState = themedTokens[0].state ? new GrammarState(
    Object.fromEntries(themedTokens.map((s2) => {
      var _a2;
      return [s2.theme, (_a2 = s2.state) == null ? void 0 : _a2.getInternalStack(s2.theme)];
    })),
    themedTokens[0].state.lang
  ) : void 0;
  if (mergedGrammarState)
    setLastGrammarStateToMap(mergedTokens, mergedGrammarState);
  return mergedTokens;
}
function syncThemesTokenization(...themes) {
  const outThemes = themes.map(() => []);
  const count = themes.length;
  for (let i2 = 0; i2 < themes[0].length; i2++) {
    const lines = themes.map((t) => t[i2]);
    const outLines = outThemes.map(() => []);
    outThemes.forEach((t, i22) => t.push(outLines[i22]));
    const indexes = lines.map(() => 0);
    const current = lines.map((l2) => l2[0]);
    while (current.every((t) => t)) {
      const minLength = Math.min(...current.map((t) => t.content.length));
      for (let n = 0; n < count; n++) {
        const token2 = current[n];
        if (token2.content.length === minLength) {
          outLines[n].push(token2);
          indexes[n] += 1;
          current[n] = lines[n][indexes[n]];
        } else {
          outLines[n].push({
            ...token2,
            content: token2.content.slice(0, minLength)
          });
          current[n] = {
            ...token2,
            content: token2.content.slice(minLength),
            offset: token2.offset + minLength
          };
        }
      }
    }
  }
  return outThemes;
}
function codeToTokens$1(internal, code, options) {
  let bg;
  let fg;
  let tokens;
  let themeName;
  let rootStyle;
  let grammarState;
  if ("themes" in options) {
    const {
      defaultColor = "light",
      cssVariablePrefix = "--shiki-",
      colorsRendering = "css-vars"
    } = options;
    const themes = Object.entries(options.themes).filter((i2) => i2[1]).map((i2) => ({ color: i2[0], theme: i2[1] })).sort((a, b2) => a.color === defaultColor ? -1 : b2.color === defaultColor ? 1 : 0);
    if (themes.length === 0)
      throw new ShikiError$2("`themes` option must not be empty");
    const themeTokens = codeToTokensWithThemes$1(
      internal,
      code,
      options
    );
    grammarState = getLastGrammarStateFromMap(themeTokens);
    if (defaultColor && DEFAULT_COLOR_LIGHT_DARK !== defaultColor && !themes.find((t) => t.color === defaultColor))
      throw new ShikiError$2(`\`themes\` option must contain the defaultColor key \`${defaultColor}\``);
    const themeRegs = themes.map((t) => internal.getTheme(t.theme));
    const themesOrder = themes.map((t) => t.color);
    tokens = themeTokens.map((line) => line.map((token2) => flatTokenVariants(token2, themesOrder, cssVariablePrefix, defaultColor, colorsRendering)));
    if (grammarState)
      setLastGrammarStateToMap(tokens, grammarState);
    const themeColorReplacements = themes.map((t) => resolveColorReplacements(t.theme, options));
    fg = mapThemeColors(themes, themeRegs, themeColorReplacements, cssVariablePrefix, defaultColor, "fg", colorsRendering);
    bg = mapThemeColors(themes, themeRegs, themeColorReplacements, cssVariablePrefix, defaultColor, "bg", colorsRendering);
    themeName = `shiki-themes ${themeRegs.map((t) => t.name).join(" ")}`;
    rootStyle = defaultColor ? void 0 : [fg, bg].join(";");
  } else if ("theme" in options) {
    const colorReplacements = resolveColorReplacements(options.theme, options);
    tokens = codeToTokensBase$1(
      internal,
      code,
      options
    );
    const _theme = internal.getTheme(options.theme);
    bg = applyColorReplacements(_theme.bg, colorReplacements);
    fg = applyColorReplacements(_theme.fg, colorReplacements);
    themeName = _theme.name;
    grammarState = getLastGrammarStateFromMap(tokens);
  } else {
    throw new ShikiError$2("Invalid options, either `theme` or `themes` must be provided");
  }
  return {
    tokens,
    fg,
    bg,
    themeName,
    rootStyle,
    grammarState
  };
}
function mapThemeColors(themes, themeRegs, themeColorReplacements, cssVariablePrefix, defaultColor, property, colorsRendering) {
  return themes.map((t, idx) => {
    const value = applyColorReplacements(themeRegs[idx][property], themeColorReplacements[idx]) || "inherit";
    const cssVar = `${cssVariablePrefix + t.color}${property === "bg" ? "-bg" : ""}:${value}`;
    if (idx === 0 && defaultColor) {
      if (defaultColor === DEFAULT_COLOR_LIGHT_DARK && themes.length > 1) {
        const lightIndex = themes.findIndex((t2) => t2.color === "light");
        const darkIndex = themes.findIndex((t2) => t2.color === "dark");
        if (lightIndex === -1 || darkIndex === -1)
          throw new ShikiError$2('When using `defaultColor: "light-dark()"`, you must provide both `light` and `dark` themes');
        const lightValue = applyColorReplacements(themeRegs[lightIndex][property], themeColorReplacements[lightIndex]) || "inherit";
        const darkValue = applyColorReplacements(themeRegs[darkIndex][property], themeColorReplacements[darkIndex]) || "inherit";
        return `light-dark(${lightValue}, ${darkValue});${cssVar}`;
      }
      return value;
    }
    if (colorsRendering === "css-vars") {
      return cssVar;
    }
    return null;
  }).filter((i2) => !!i2).join(";");
}
function codeToHast$1(internal, code, options, transformerContext = {
  meta: {},
  options,
  codeToHast: (_code, _options) => codeToHast$1(internal, _code, _options),
  codeToTokens: (_code, _options) => codeToTokens$1(internal, _code, _options)
}) {
  var _a2, _b2;
  let input = code;
  for (const transformer of getTransformers(options))
    input = ((_a2 = transformer.preprocess) == null ? void 0 : _a2.call(transformerContext, input, options)) || input;
  let {
    tokens,
    fg,
    bg,
    themeName,
    rootStyle,
    grammarState
  } = codeToTokens$1(internal, input, options);
  const {
    mergeWhitespaces = true,
    mergeSameStyleTokens = false
  } = options;
  if (mergeWhitespaces === true)
    tokens = mergeWhitespaceTokens(tokens);
  else if (mergeWhitespaces === "never")
    tokens = splitWhitespaceTokens(tokens);
  if (mergeSameStyleTokens) {
    tokens = mergeAdjacentStyledTokens(tokens);
  }
  const contextSource = {
    ...transformerContext,
    get source() {
      return input;
    }
  };
  for (const transformer of getTransformers(options))
    tokens = ((_b2 = transformer.tokens) == null ? void 0 : _b2.call(contextSource, tokens)) || tokens;
  return tokensToHast(
    tokens,
    {
      ...options,
      fg,
      bg,
      themeName,
      rootStyle: options.rootStyle === false ? false : options.rootStyle ?? rootStyle
    },
    contextSource,
    grammarState
  );
}
function tokensToHast(tokens, options, transformerContext, grammarState = getLastGrammarStateFromMap(tokens)) {
  var _a2, _b2, _c2, _d;
  const transformers = getTransformers(options);
  const lines = [];
  const root2 = {
    type: "root",
    children: []
  };
  const {
    structure = "classic",
    tabindex = "0"
  } = options;
  const properties = {
    class: `shiki ${options.themeName || ""}`
  };
  if (options.rootStyle !== false) {
    if (options.rootStyle != null)
      properties.style = options.rootStyle;
    else
      properties.style = `background-color:${options.bg};color:${options.fg}`;
  }
  if (tabindex !== false && tabindex != null)
    properties.tabindex = tabindex.toString();
  for (const [key2, value] of Object.entries(options.meta || {})) {
    if (!key2.startsWith("_"))
      properties[key2] = value;
  }
  let preNode = {
    type: "element",
    tagName: "pre",
    properties,
    children: [],
    data: options.data
  };
  let codeNode = {
    type: "element",
    tagName: "code",
    properties: {},
    children: lines
  };
  const lineNodes = [];
  const context = {
    ...transformerContext,
    structure,
    addClassToHast,
    get source() {
      return transformerContext.source;
    },
    get tokens() {
      return tokens;
    },
    get options() {
      return options;
    },
    get root() {
      return root2;
    },
    get pre() {
      return preNode;
    },
    get code() {
      return codeNode;
    },
    get lines() {
      return lineNodes;
    }
  };
  tokens.forEach((line, idx) => {
    var _a3, _b3;
    if (idx) {
      if (structure === "inline")
        root2.children.push({ type: "element", tagName: "br", properties: {}, children: [] });
      else if (structure === "classic")
        lines.push({ type: "text", value: "\n" });
    }
    let lineNode = {
      type: "element",
      tagName: "span",
      properties: { class: "line" },
      children: []
    };
    let col = 0;
    for (const token2 of line) {
      let tokenNode = {
        type: "element",
        tagName: "span",
        properties: {
          ...token2.htmlAttrs
        },
        children: [{ type: "text", value: token2.content }]
      };
      const style = stringifyTokenStyle(token2.htmlStyle || getTokenStyleObject(token2));
      if (style)
        tokenNode.properties.style = style;
      for (const transformer of transformers)
        tokenNode = ((_a3 = transformer == null ? void 0 : transformer.span) == null ? void 0 : _a3.call(context, tokenNode, idx + 1, col, lineNode, token2)) || tokenNode;
      if (structure === "inline")
        root2.children.push(tokenNode);
      else if (structure === "classic")
        lineNode.children.push(tokenNode);
      col += token2.content.length;
    }
    if (structure === "classic") {
      for (const transformer of transformers)
        lineNode = ((_b3 = transformer == null ? void 0 : transformer.line) == null ? void 0 : _b3.call(context, lineNode, idx + 1)) || lineNode;
      lineNodes.push(lineNode);
      lines.push(lineNode);
    } else if (structure === "inline") {
      lineNodes.push(lineNode);
    }
  });
  if (structure === "classic") {
    for (const transformer of transformers)
      codeNode = ((_a2 = transformer == null ? void 0 : transformer.code) == null ? void 0 : _a2.call(context, codeNode)) || codeNode;
    preNode.children.push(codeNode);
    for (const transformer of transformers)
      preNode = ((_b2 = transformer == null ? void 0 : transformer.pre) == null ? void 0 : _b2.call(context, preNode)) || preNode;
    root2.children.push(preNode);
  } else if (structure === "inline") {
    const syntheticLines = [];
    let currentLine = {
      type: "element",
      tagName: "span",
      properties: { class: "line" },
      children: []
    };
    for (const child of root2.children) {
      if (child.type === "element" && child.tagName === "br") {
        syntheticLines.push(currentLine);
        currentLine = {
          type: "element",
          tagName: "span",
          properties: { class: "line" },
          children: []
        };
      } else if (child.type === "element" || child.type === "text") {
        currentLine.children.push(child);
      }
    }
    syntheticLines.push(currentLine);
    const syntheticCode = {
      type: "element",
      tagName: "code",
      properties: {},
      children: syntheticLines
    };
    let transformedCode = syntheticCode;
    for (const transformer of transformers)
      transformedCode = ((_c2 = transformer == null ? void 0 : transformer.code) == null ? void 0 : _c2.call(context, transformedCode)) || transformedCode;
    root2.children = [];
    for (let i2 = 0; i2 < transformedCode.children.length; i2++) {
      if (i2 > 0)
        root2.children.push({ type: "element", tagName: "br", properties: {}, children: [] });
      const line = transformedCode.children[i2];
      if (line.type === "element")
        root2.children.push(...line.children);
    }
  }
  let result = root2;
  for (const transformer of transformers)
    result = ((_d = transformer == null ? void 0 : transformer.root) == null ? void 0 : _d.call(context, result)) || result;
  if (grammarState)
    setLastGrammarStateToMap(result, grammarState);
  return result;
}
function mergeWhitespaceTokens(tokens) {
  return tokens.map((line) => {
    const newLine = [];
    let carryOnContent = "";
    let firstOffset;
    line.forEach((token2, idx) => {
      const isDecorated = token2.fontStyle && (token2.fontStyle & FontStyle.Underline || token2.fontStyle & FontStyle.Strikethrough);
      const couldMerge = !isDecorated;
      if (couldMerge && token2.content.match(/^\s+$/) && line[idx + 1]) {
        if (firstOffset === void 0)
          firstOffset = token2.offset;
        carryOnContent += token2.content;
      } else {
        if (carryOnContent) {
          if (couldMerge) {
            newLine.push({
              ...token2,
              offset: firstOffset,
              content: carryOnContent + token2.content
            });
          } else {
            newLine.push(
              {
                content: carryOnContent,
                offset: firstOffset
              },
              token2
            );
          }
          firstOffset = void 0;
          carryOnContent = "";
        } else {
          newLine.push(token2);
        }
      }
    });
    return newLine;
  });
}
function splitWhitespaceTokens(tokens) {
  return tokens.map((line) => {
    return line.flatMap((token2) => {
      if (token2.content.match(/^\s+$/))
        return token2;
      const match = token2.content.match(/^(\s*)(.*?)(\s*)$/);
      if (!match)
        return token2;
      const [, leading, content, trailing] = match;
      if (!leading && !trailing)
        return token2;
      const expanded = [{
        ...token2,
        offset: token2.offset + leading.length,
        content
      }];
      if (leading) {
        expanded.unshift({
          content: leading,
          offset: token2.offset
        });
      }
      if (trailing) {
        expanded.push({
          content: trailing,
          offset: token2.offset + leading.length + content.length
        });
      }
      return expanded;
    });
  });
}
function mergeAdjacentStyledTokens(tokens) {
  return tokens.map((line) => {
    const newLine = [];
    for (const token2 of line) {
      if (newLine.length === 0) {
        newLine.push({ ...token2 });
        continue;
      }
      const prevToken = newLine[newLine.length - 1];
      const prevStyle = stringifyTokenStyle(prevToken.htmlStyle || getTokenStyleObject(prevToken));
      const currentStyle = stringifyTokenStyle(token2.htmlStyle || getTokenStyleObject(token2));
      const isPrevDecorated = prevToken.fontStyle && (prevToken.fontStyle & FontStyle.Underline || prevToken.fontStyle & FontStyle.Strikethrough);
      const isDecorated = token2.fontStyle && (token2.fontStyle & FontStyle.Underline || token2.fontStyle & FontStyle.Strikethrough);
      if (!isPrevDecorated && !isDecorated && prevStyle === currentStyle) {
        prevToken.content += token2.content;
      } else {
        newLine.push({ ...token2 });
      }
    }
    return newLine;
  });
}
const hastToHtml = toHtml;
function codeToHtml$1(internal, code, options) {
  var _a2;
  const context = {
    meta: {},
    options,
    codeToHast: (_code, _options) => codeToHast$1(internal, _code, _options),
    codeToTokens: (_code, _options) => codeToTokens$1(internal, _code, _options)
  };
  let result = hastToHtml(codeToHast$1(internal, code, options, context));
  for (const transformer of getTransformers(options))
    result = ((_a2 = transformer.postprocess) == null ? void 0 : _a2.call(context, result, options)) || result;
  return result;
}
const VSCODE_FALLBACK_EDITOR_FG = { light: "#333333", dark: "#bbbbbb" };
const VSCODE_FALLBACK_EDITOR_BG = { light: "#fffffe", dark: "#1e1e1e" };
const RESOLVED_KEY = "__shiki_resolved";
function normalizeTheme(rawTheme) {
  var _a2, _b2, _c2, _d, _e;
  if (rawTheme == null ? void 0 : rawTheme[RESOLVED_KEY])
    return rawTheme;
  const theme = {
    ...rawTheme
  };
  if (theme.tokenColors && !theme.settings) {
    theme.settings = theme.tokenColors;
    delete theme.tokenColors;
  }
  theme.type || (theme.type = "dark");
  theme.colorReplacements = { ...theme.colorReplacements };
  theme.settings || (theme.settings = []);
  let { bg, fg } = theme;
  if (!bg || !fg) {
    const globalSetting = theme.settings ? theme.settings.find((s2) => !s2.name && !s2.scope) : void 0;
    if ((_a2 = globalSetting == null ? void 0 : globalSetting.settings) == null ? void 0 : _a2.foreground)
      fg = globalSetting.settings.foreground;
    if ((_b2 = globalSetting == null ? void 0 : globalSetting.settings) == null ? void 0 : _b2.background)
      bg = globalSetting.settings.background;
    if (!fg && ((_c2 = theme == null ? void 0 : theme.colors) == null ? void 0 : _c2["editor.foreground"]))
      fg = theme.colors["editor.foreground"];
    if (!bg && ((_d = theme == null ? void 0 : theme.colors) == null ? void 0 : _d["editor.background"]))
      bg = theme.colors["editor.background"];
    if (!fg)
      fg = theme.type === "light" ? VSCODE_FALLBACK_EDITOR_FG.light : VSCODE_FALLBACK_EDITOR_FG.dark;
    if (!bg)
      bg = theme.type === "light" ? VSCODE_FALLBACK_EDITOR_BG.light : VSCODE_FALLBACK_EDITOR_BG.dark;
    theme.fg = fg;
    theme.bg = bg;
  }
  if (!(theme.settings[0] && theme.settings[0].settings && !theme.settings[0].scope)) {
    theme.settings.unshift({
      settings: {
        foreground: theme.fg,
        background: theme.bg
      }
    });
  }
  let replacementCount = 0;
  const replacementMap = /* @__PURE__ */ new Map();
  function getReplacementColor(value) {
    var _a3;
    if (replacementMap.has(value))
      return replacementMap.get(value);
    replacementCount += 1;
    const hex = `#${replacementCount.toString(16).padStart(8, "0").toLowerCase()}`;
    if ((_a3 = theme.colorReplacements) == null ? void 0 : _a3[`#${hex}`])
      return getReplacementColor(value);
    replacementMap.set(value, hex);
    return hex;
  }
  theme.settings = theme.settings.map((setting) => {
    var _a3, _b3;
    const replaceFg = ((_a3 = setting.settings) == null ? void 0 : _a3.foreground) && !setting.settings.foreground.startsWith("#");
    const replaceBg = ((_b3 = setting.settings) == null ? void 0 : _b3.background) && !setting.settings.background.startsWith("#");
    if (!replaceFg && !replaceBg)
      return setting;
    const clone2 = {
      ...setting,
      settings: {
        ...setting.settings
      }
    };
    if (replaceFg) {
      const replacement = getReplacementColor(setting.settings.foreground);
      theme.colorReplacements[replacement] = setting.settings.foreground;
      clone2.settings.foreground = replacement;
    }
    if (replaceBg) {
      const replacement = getReplacementColor(setting.settings.background);
      theme.colorReplacements[replacement] = setting.settings.background;
      clone2.settings.background = replacement;
    }
    return clone2;
  });
  for (const key2 of Object.keys(theme.colors || {})) {
    if (key2 === "editor.foreground" || key2 === "editor.background" || key2.startsWith("terminal.ansi")) {
      if (!((_e = theme.colors[key2]) == null ? void 0 : _e.startsWith("#"))) {
        const replacement = getReplacementColor(theme.colors[key2]);
        theme.colorReplacements[replacement] = theme.colors[key2];
        theme.colors[key2] = replacement;
      }
    }
  }
  Object.defineProperty(theme, RESOLVED_KEY, {
    enumerable: false,
    writable: false,
    value: true
  });
  return theme;
}
async function resolveLangs(langs) {
  return Array.from(new Set((await Promise.all(
    langs.filter((l2) => !isSpecialLang(l2)).map(async (lang) => await normalizeGetter(lang).then((r2) => Array.isArray(r2) ? r2 : [r2]))
  )).flat()));
}
async function resolveThemes(themes) {
  const resolved = await Promise.all(
    themes.map(
      async (theme) => isSpecialTheme(theme) ? null : normalizeTheme(await normalizeGetter(theme))
    )
  );
  return resolved.filter((i2) => !!i2);
}
let _emitDeprecation = 3;
let _emitError = false;
function enableDeprecationWarnings(emitDeprecation = true, emitError = false) {
  _emitDeprecation = emitDeprecation;
  _emitError = emitError;
}
function warnDeprecated(message, version = 3) {
  if (!_emitDeprecation)
    return;
  if (typeof _emitDeprecation === "number" && version > _emitDeprecation)
    return;
  if (_emitError) {
    throw new Error(`[SHIKI DEPRECATE]: ${message}`);
  } else {
    console.trace(`[SHIKI DEPRECATE]: ${message}`);
  }
}
let ShikiError$1 = class ShikiError2 extends Error {
  constructor(message) {
    super(message);
    this.name = "ShikiError";
  }
};
function resolveLangAlias(name, alias) {
  if (!alias)
    return name;
  if (alias[name]) {
    const resolved = /* @__PURE__ */ new Set([name]);
    while (alias[name]) {
      name = alias[name];
      if (resolved.has(name))
        throw new ShikiError$1(`Circular alias \`${Array.from(resolved).join(" -> ")} -> ${name}\``);
      resolved.add(name);
    }
  }
  return name;
}
class Registry2 extends Registry$1 {
  constructor(_resolver, _themes, _langs, _alias = {}) {
    super(_resolver);
    __publicField(this, "_resolvedThemes", /* @__PURE__ */ new Map());
    __publicField(this, "_resolvedGrammars", /* @__PURE__ */ new Map());
    __publicField(this, "_langMap", /* @__PURE__ */ new Map());
    __publicField(this, "_langGraph", /* @__PURE__ */ new Map());
    __publicField(this, "_textmateThemeCache", /* @__PURE__ */ new WeakMap());
    __publicField(this, "_loadedThemesCache", null);
    __publicField(this, "_loadedLanguagesCache", null);
    this._resolver = _resolver;
    this._themes = _themes;
    this._langs = _langs;
    this._alias = _alias;
    this._themes.map((t) => this.loadTheme(t));
    this.loadLanguages(this._langs);
  }
  getTheme(theme) {
    if (typeof theme === "string")
      return this._resolvedThemes.get(theme);
    else
      return this.loadTheme(theme);
  }
  loadTheme(theme) {
    const _theme = normalizeTheme(theme);
    if (_theme.name) {
      this._resolvedThemes.set(_theme.name, _theme);
      this._loadedThemesCache = null;
    }
    return _theme;
  }
  getLoadedThemes() {
    if (!this._loadedThemesCache)
      this._loadedThemesCache = [...this._resolvedThemes.keys()];
    return this._loadedThemesCache;
  }
  // Override and re-implement this method to cache the textmate themes as `TextMateTheme.createFromRawTheme`
  // is expensive. Themes can switch often especially for dual-theme support.
  //
  // The parent class also accepts `colorMap` as the second parameter, but since we don't use that,
  // we omit here so it's easier to cache the themes.
  setTheme(theme) {
    let textmateTheme = this._textmateThemeCache.get(theme);
    if (!textmateTheme) {
      textmateTheme = Theme.createFromRawTheme(theme);
      this._textmateThemeCache.set(theme, textmateTheme);
    }
    this._syncRegistry.setTheme(textmateTheme);
  }
  getGrammar(name) {
    name = resolveLangAlias(name, this._alias);
    return this._resolvedGrammars.get(name);
  }
  loadLanguage(lang) {
    var _a2, _b2, _c2, _d;
    if (this.getGrammar(lang.name))
      return;
    const embeddedLazilyBy = new Set(
      [...this._langMap.values()].filter((i2) => {
        var _a3;
        return (_a3 = i2.embeddedLangsLazy) == null ? void 0 : _a3.includes(lang.name);
      })
    );
    this._resolver.addLanguage(lang);
    const grammarConfig = {
      balancedBracketSelectors: lang.balancedBracketSelectors || ["*"],
      unbalancedBracketSelectors: lang.unbalancedBracketSelectors || []
    };
    this._syncRegistry._rawGrammars.set(lang.scopeName, lang);
    const g = this.loadGrammarWithConfiguration(lang.scopeName, 1, grammarConfig);
    g.name = lang.name;
    this._resolvedGrammars.set(lang.name, g);
    if (lang.aliases) {
      lang.aliases.forEach((alias) => {
        this._alias[alias] = lang.name;
      });
    }
    this._loadedLanguagesCache = null;
    if (embeddedLazilyBy.size) {
      for (const e of embeddedLazilyBy) {
        this._resolvedGrammars.delete(e.name);
        this._loadedLanguagesCache = null;
        (_b2 = (_a2 = this._syncRegistry) == null ? void 0 : _a2._injectionGrammars) == null ? void 0 : _b2.delete(e.scopeName);
        (_d = (_c2 = this._syncRegistry) == null ? void 0 : _c2._grammars) == null ? void 0 : _d.delete(e.scopeName);
        this.loadLanguage(this._langMap.get(e.name));
      }
    }
  }
  dispose() {
    super.dispose();
    this._resolvedThemes.clear();
    this._resolvedGrammars.clear();
    this._langMap.clear();
    this._langGraph.clear();
    this._loadedThemesCache = null;
  }
  loadLanguages(langs) {
    for (const lang of langs)
      this.resolveEmbeddedLanguages(lang);
    const langsGraphArray = Array.from(this._langGraph.entries());
    const missingLangs = langsGraphArray.filter(([_2, lang]) => !lang);
    if (missingLangs.length) {
      const dependents = langsGraphArray.filter(([_2, lang]) => {
        if (!lang)
          return false;
        const embedded = lang.embeddedLanguages || lang.embeddedLangs;
        return embedded == null ? void 0 : embedded.some((l2) => missingLangs.map(([name]) => name).includes(l2));
      }).filter((lang) => !missingLangs.includes(lang));
      throw new ShikiError$1(`Missing languages ${missingLangs.map(([name]) => `\`${name}\``).join(", ")}, required by ${dependents.map(([name]) => `\`${name}\``).join(", ")}`);
    }
    for (const [_2, lang] of langsGraphArray)
      this._resolver.addLanguage(lang);
    for (const [_2, lang] of langsGraphArray)
      this.loadLanguage(lang);
  }
  getLoadedLanguages() {
    if (!this._loadedLanguagesCache) {
      this._loadedLanguagesCache = [
        .../* @__PURE__ */ new Set([...this._resolvedGrammars.keys(), ...Object.keys(this._alias)])
      ];
    }
    return this._loadedLanguagesCache;
  }
  resolveEmbeddedLanguages(lang) {
    this._langMap.set(lang.name, lang);
    this._langGraph.set(lang.name, lang);
    const embedded = lang.embeddedLanguages ?? lang.embeddedLangs;
    if (embedded) {
      for (const embeddedLang of embedded)
        this._langGraph.set(embeddedLang, this._langMap.get(embeddedLang));
    }
  }
}
class Resolver {
  constructor(engine, langs) {
    __publicField(this, "_langs", /* @__PURE__ */ new Map());
    __publicField(this, "_scopeToLang", /* @__PURE__ */ new Map());
    __publicField(this, "_injections", /* @__PURE__ */ new Map());
    __publicField(this, "_onigLib");
    this._onigLib = {
      createOnigScanner: (patterns) => engine.createScanner(patterns),
      createOnigString: (s2) => engine.createString(s2)
    };
    langs.forEach((i2) => this.addLanguage(i2));
  }
  get onigLib() {
    return this._onigLib;
  }
  getLangRegistration(langIdOrAlias) {
    return this._langs.get(langIdOrAlias);
  }
  loadGrammar(scopeName) {
    return this._scopeToLang.get(scopeName);
  }
  addLanguage(l2) {
    this._langs.set(l2.name, l2);
    if (l2.aliases) {
      l2.aliases.forEach((a) => {
        this._langs.set(a, l2);
      });
    }
    this._scopeToLang.set(l2.scopeName, l2);
    if (l2.injectTo) {
      l2.injectTo.forEach((i2) => {
        if (!this._injections.get(i2))
          this._injections.set(i2, []);
        this._injections.get(i2).push(l2.scopeName);
      });
    }
  }
  getInjections(scopeName) {
    const scopeParts = scopeName.split(".");
    let injections = [];
    for (let i2 = 1; i2 <= scopeParts.length; i2++) {
      const subScopeName = scopeParts.slice(0, i2).join(".");
      injections = [...injections, ...this._injections.get(subScopeName) || []];
    }
    return injections;
  }
}
let instancesCount = 0;
function createShikiInternalSync(options) {
  instancesCount += 1;
  if (options.warnings !== false && instancesCount >= 10 && instancesCount % 10 === 0)
    console.warn(`[Shiki] ${instancesCount} instances have been created. Shiki is supposed to be used as a singleton, consider refactoring your code to cache your highlighter instance; Or call \`highlighter.dispose()\` to release unused instances.`);
  let isDisposed = false;
  if (!options.engine)
    throw new ShikiError$1("`engine` option is required for synchronous mode");
  const langs = (options.langs || []).flat(1);
  const themes = (options.themes || []).flat(1).map(normalizeTheme);
  const resolver = new Resolver(options.engine, langs);
  const _registry = new Registry2(resolver, themes, langs, options.langAlias);
  let _lastTheme;
  function resolveLangAlias$1(name) {
    return resolveLangAlias(name, options.langAlias);
  }
  function getLanguage(name) {
    ensureNotDisposed();
    const _lang = _registry.getGrammar(typeof name === "string" ? name : name.name);
    if (!_lang)
      throw new ShikiError$1(`Language \`${name}\` not found, you may need to load it first`);
    return _lang;
  }
  function getTheme(name) {
    if (name === "none")
      return { bg: "", fg: "", name: "none", settings: [], type: "dark" };
    ensureNotDisposed();
    const _theme = _registry.getTheme(name);
    if (!_theme)
      throw new ShikiError$1(`Theme \`${name}\` not found, you may need to load it first`);
    return _theme;
  }
  function setTheme(name) {
    ensureNotDisposed();
    const theme = getTheme(name);
    if (_lastTheme !== name) {
      _registry.setTheme(theme);
      _lastTheme = name;
    }
    const colorMap = _registry.getColorMap();
    return {
      theme,
      colorMap
    };
  }
  function getLoadedThemes() {
    ensureNotDisposed();
    return _registry.getLoadedThemes();
  }
  function getLoadedLanguages() {
    ensureNotDisposed();
    return _registry.getLoadedLanguages();
  }
  function loadLanguageSync(...langs2) {
    ensureNotDisposed();
    _registry.loadLanguages(langs2.flat(1));
  }
  async function loadLanguage(...langs2) {
    return loadLanguageSync(await resolveLangs(langs2));
  }
  function loadThemeSync(...themes2) {
    ensureNotDisposed();
    for (const theme of themes2.flat(1)) {
      _registry.loadTheme(theme);
    }
  }
  async function loadTheme(...themes2) {
    ensureNotDisposed();
    return loadThemeSync(await resolveThemes(themes2));
  }
  function ensureNotDisposed() {
    if (isDisposed)
      throw new ShikiError$1("Shiki instance has been disposed");
  }
  function dispose() {
    if (isDisposed)
      return;
    isDisposed = true;
    _registry.dispose();
    instancesCount -= 1;
  }
  return {
    setTheme,
    getTheme,
    getLanguage,
    getLoadedThemes,
    getLoadedLanguages,
    resolveLangAlias: resolveLangAlias$1,
    loadLanguage,
    loadLanguageSync,
    loadTheme,
    loadThemeSync,
    dispose,
    [Symbol.dispose]: dispose
  };
}
async function createShikiInternal(options) {
  if (!options.engine) {
    warnDeprecated("`engine` option is required. Use `createOnigurumaEngine` or `createJavaScriptRegexEngine` to create an engine.");
  }
  const [
    themes,
    langs,
    engine
  ] = await Promise.all([
    resolveThemes(options.themes || []),
    resolveLangs(options.langs || []),
    options.engine
  ]);
  return createShikiInternalSync({
    ...options,
    themes,
    langs,
    engine
  });
}
async function createHighlighterCore(options) {
  const internal = await createShikiInternal(options);
  return {
    getLastGrammarState: (...args) => getLastGrammarState$1(internal, ...args),
    codeToTokensBase: (code, options2) => codeToTokensBase$1(internal, code, options2),
    codeToTokensWithThemes: (code, options2) => codeToTokensWithThemes$1(internal, code, options2),
    codeToTokens: (code, options2) => codeToTokens$1(internal, code, options2),
    codeToHast: (code, options2) => codeToHast$1(internal, code, options2),
    codeToHtml: (code, options2) => codeToHtml$1(internal, code, options2),
    getBundledLanguages: () => ({}),
    getBundledThemes: () => ({}),
    ...internal,
    getInternalContext: () => internal
  };
}
function createHighlighterCoreSync(options) {
  const internal = createShikiInternalSync(options);
  return {
    getLastGrammarState: (...args) => getLastGrammarState$1(internal, ...args),
    codeToTokensBase: (code, options2) => codeToTokensBase$1(internal, code, options2),
    codeToTokensWithThemes: (code, options2) => codeToTokensWithThemes$1(internal, code, options2),
    codeToTokens: (code, options2) => codeToTokens$1(internal, code, options2),
    codeToHast: (code, options2) => codeToHast$1(internal, code, options2),
    codeToHtml: (code, options2) => codeToHtml$1(internal, code, options2),
    getBundledLanguages: () => ({}),
    getBundledThemes: () => ({}),
    ...internal,
    getInternalContext: () => internal
  };
}
function makeSingletonHighlighterCore(createHighlighter2) {
  let _shiki;
  async function getSingletonHighlighterCore2(options) {
    if (!_shiki) {
      _shiki = createHighlighter2({
        ...options,
        themes: options.themes || [],
        langs: options.langs || []
      });
      return _shiki;
    } else {
      const s2 = await _shiki;
      await Promise.all([
        s2.loadTheme(...options.themes || []),
        s2.loadLanguage(...options.langs || [])
      ]);
      return s2;
    }
  }
  return getSingletonHighlighterCore2;
}
const getSingletonHighlighterCore = /* @__PURE__ */ makeSingletonHighlighterCore(createHighlighterCore);
function createBundledHighlighter(options) {
  const bundledLanguages2 = options.langs;
  const bundledThemes2 = options.themes;
  const engine = options.engine;
  async function createHighlighter2(options2) {
    function resolveLang(lang) {
      var _a2;
      if (typeof lang === "string") {
        lang = ((_a2 = options2.langAlias) == null ? void 0 : _a2[lang]) || lang;
        if (isSpecialLang(lang))
          return [];
        const bundle = bundledLanguages2[lang];
        if (!bundle)
          throw new ShikiError$2(`Language \`${lang}\` is not included in this bundle. You may want to load it from external source.`);
        return bundle;
      }
      return lang;
    }
    function resolveTheme(theme) {
      if (isSpecialTheme(theme))
        return "none";
      if (typeof theme === "string") {
        const bundle = bundledThemes2[theme];
        if (!bundle)
          throw new ShikiError$2(`Theme \`${theme}\` is not included in this bundle. You may want to load it from external source.`);
        return bundle;
      }
      return theme;
    }
    const _themes = (options2.themes ?? []).map((i2) => resolveTheme(i2));
    const langs = (options2.langs ?? []).map((i2) => resolveLang(i2));
    const core2 = await createHighlighterCore({
      engine: options2.engine ?? engine(),
      ...options2,
      themes: _themes,
      langs
    });
    return {
      ...core2,
      loadLanguage(...langs2) {
        return core2.loadLanguage(...langs2.map(resolveLang));
      },
      loadTheme(...themes) {
        return core2.loadTheme(...themes.map(resolveTheme));
      },
      getBundledLanguages() {
        return bundledLanguages2;
      },
      getBundledThemes() {
        return bundledThemes2;
      }
    };
  }
  return createHighlighter2;
}
function makeSingletonHighlighter(createHighlighter2) {
  let _shiki;
  async function getSingletonHighlighter2(options = {}) {
    if (!_shiki) {
      _shiki = createHighlighter2({
        ...options,
        themes: [],
        langs: []
      });
      const s2 = await _shiki;
      await Promise.all([
        s2.loadTheme(...options.themes || []),
        s2.loadLanguage(...options.langs || [])
      ]);
      return s2;
    } else {
      const s2 = await _shiki;
      await Promise.all([
        s2.loadTheme(...options.themes || []),
        s2.loadLanguage(...options.langs || [])
      ]);
      return s2;
    }
  }
  return getSingletonHighlighter2;
}
function createSingletonShorthands(createHighlighter2, config) {
  const getSingletonHighlighter2 = makeSingletonHighlighter(createHighlighter2);
  async function get(code, options) {
    var _a2;
    const shiki = await getSingletonHighlighter2({
      langs: [options.lang],
      themes: "theme" in options ? [options.theme] : Object.values(options.themes)
    });
    const langs = await ((_a2 = config == null ? void 0 : config.guessEmbeddedLanguages) == null ? void 0 : _a2.call(config, code, options.lang, shiki));
    if (langs) {
      await shiki.loadLanguage(...langs);
    }
    return shiki;
  }
  return {
    getSingletonHighlighter(options) {
      return getSingletonHighlighter2(options);
    },
    async codeToHtml(code, options) {
      const shiki = await get(code, options);
      return shiki.codeToHtml(code, options);
    },
    async codeToHast(code, options) {
      const shiki = await get(code, options);
      return shiki.codeToHast(code, options);
    },
    async codeToTokens(code, options) {
      const shiki = await get(code, options);
      return shiki.codeToTokens(code, options);
    },
    async codeToTokensBase(code, options) {
      const shiki = await get(code, options);
      return shiki.codeToTokensBase(code, options);
    },
    async codeToTokensWithThemes(code, options) {
      const shiki = await get(code, options);
      return shiki.codeToTokensWithThemes(code, options);
    },
    async getLastGrammarState(code, options) {
      const shiki = await getSingletonHighlighter2({
        langs: [options.lang],
        themes: [options.theme]
      });
      return shiki.getLastGrammarState(code, options);
    }
  };
}
const createdBundledHighlighter = createBundledHighlighter;
function createCssVariablesTheme(options = {}) {
  var _a2;
  const {
    name = "css-variables",
    variablePrefix = "--shiki-",
    fontStyle = true
  } = options;
  const variable = (name2) => {
    var _a3;
    if ((_a3 = options.variableDefaults) == null ? void 0 : _a3[name2])
      return `var(${variablePrefix}${name2}, ${options.variableDefaults[name2]})`;
    return `var(${variablePrefix}${name2})`;
  };
  const theme = {
    name,
    type: "dark",
    colors: {
      "editor.foreground": variable("foreground"),
      "editor.background": variable("background"),
      "terminal.ansiBlack": variable("ansi-black"),
      "terminal.ansiRed": variable("ansi-red"),
      "terminal.ansiGreen": variable("ansi-green"),
      "terminal.ansiYellow": variable("ansi-yellow"),
      "terminal.ansiBlue": variable("ansi-blue"),
      "terminal.ansiMagenta": variable("ansi-magenta"),
      "terminal.ansiCyan": variable("ansi-cyan"),
      "terminal.ansiWhite": variable("ansi-white"),
      "terminal.ansiBrightBlack": variable("ansi-bright-black"),
      "terminal.ansiBrightRed": variable("ansi-bright-red"),
      "terminal.ansiBrightGreen": variable("ansi-bright-green"),
      "terminal.ansiBrightYellow": variable("ansi-bright-yellow"),
      "terminal.ansiBrightBlue": variable("ansi-bright-blue"),
      "terminal.ansiBrightMagenta": variable("ansi-bright-magenta"),
      "terminal.ansiBrightCyan": variable("ansi-bright-cyan"),
      "terminal.ansiBrightWhite": variable("ansi-bright-white")
    },
    tokenColors: [
      {
        scope: [
          "keyword.operator.accessor",
          "meta.group.braces.round.function.arguments",
          "meta.template.expression",
          "markup.fenced_code meta.embedded.block"
        ],
        settings: {
          foreground: variable("foreground")
        }
      },
      {
        scope: "emphasis",
        settings: {
          fontStyle: "italic"
        }
      },
      {
        scope: ["strong", "markup.heading.markdown", "markup.bold.markdown"],
        settings: {
          fontStyle: "bold"
        }
      },
      {
        scope: ["markup.italic.markdown"],
        settings: {
          fontStyle: "italic"
        }
      },
      {
        scope: "meta.link.inline.markdown",
        settings: {
          fontStyle: "underline",
          foreground: variable("token-link")
        }
      },
      {
        scope: ["string", "markup.fenced_code", "markup.inline"],
        settings: {
          foreground: variable("token-string")
        }
      },
      {
        scope: ["comment", "string.quoted.docstring.multi"],
        settings: {
          foreground: variable("token-comment")
        }
      },
      {
        scope: [
          "constant.numeric",
          "constant.language",
          "constant.other.placeholder",
          "constant.character.format.placeholder",
          "variable.language.this",
          "variable.other.object",
          "variable.other.class",
          "variable.other.constant",
          "meta.property-name",
          "meta.property-value",
          "support"
        ],
        settings: {
          foreground: variable("token-constant")
        }
      },
      {
        scope: [
          "keyword",
          "storage.modifier",
          "storage.type",
          "storage.control.clojure",
          "entity.name.function.clojure",
          "entity.name.tag.yaml",
          "support.function.node",
          "support.type.property-name.json",
          "punctuation.separator.key-value",
          "punctuation.definition.template-expression"
        ],
        settings: {
          foreground: variable("token-keyword")
        }
      },
      {
        scope: "variable.parameter.function",
        settings: {
          foreground: variable("token-parameter")
        }
      },
      {
        scope: [
          "support.function",
          "entity.name.type",
          "entity.other.inherited-class",
          "meta.function-call",
          "meta.instance.constructor",
          "entity.other.attribute-name",
          "entity.name.function",
          "constant.keyword.clojure"
        ],
        settings: {
          foreground: variable("token-function")
        }
      },
      {
        scope: [
          "entity.name.tag",
          "string.quoted",
          "string.regexp",
          "string.interpolated",
          "string.template",
          "string.unquoted.plain.out.yaml",
          "keyword.other.template"
        ],
        settings: {
          foreground: variable("token-string-expression")
        }
      },
      {
        scope: [
          "punctuation.definition.arguments",
          "punctuation.definition.dict",
          "punctuation.separator",
          "meta.function-call.arguments"
        ],
        settings: {
          foreground: variable("token-punctuation")
        }
      },
      {
        // [Custom] Markdown links
        scope: [
          "markup.underline.link",
          "punctuation.definition.metadata.markdown"
        ],
        settings: {
          foreground: variable("token-link")
        }
      },
      {
        // [Custom] Markdown list
        scope: ["beginning.punctuation.definition.list.markdown"],
        settings: {
          foreground: variable("token-string")
        }
      },
      {
        // [Custom] Markdown punctuation definition brackets
        scope: [
          "punctuation.definition.string.begin.markdown",
          "punctuation.definition.string.end.markdown",
          "string.other.link.title.markdown",
          "string.other.link.description.markdown"
        ],
        settings: {
          foreground: variable("token-keyword")
        }
      },
      {
        // [Custom] Diff
        scope: [
          "markup.inserted",
          "meta.diff.header.to-file",
          "punctuation.definition.inserted"
        ],
        settings: {
          foreground: variable("token-inserted")
        }
      },
      {
        scope: [
          "markup.deleted",
          "meta.diff.header.from-file",
          "punctuation.definition.deleted"
        ],
        settings: {
          foreground: variable("token-deleted")
        }
      },
      {
        scope: [
          "markup.changed",
          "punctuation.definition.changed"
        ],
        settings: {
          foreground: variable("token-changed")
        }
      }
    ]
  };
  if (!fontStyle) {
    theme.tokenColors = (_a2 = theme.tokenColors) == null ? void 0 : _a2.map((tokenColor) => {
      var _a3;
      if ((_a3 = tokenColor.settings) == null ? void 0 : _a3.fontStyle)
        delete tokenColor.settings.fontStyle;
      return tokenColor;
    });
  }
  return theme;
}
const bundledLanguagesInfo = [
  {
    "id": "abap",
    "name": "ABAP",
    "import": (() => __vitePreload(() => import("./abap.js"), true ? [] : void 0))
  },
  {
    "id": "actionscript-3",
    "name": "ActionScript",
    "import": (() => __vitePreload(() => import("./actionscript-3.js"), true ? [] : void 0))
  },
  {
    "id": "ada",
    "name": "Ada",
    "import": (() => __vitePreload(() => import("./ada.js"), true ? [] : void 0))
  },
  {
    "id": "angular-html",
    "name": "Angular HTML",
    "import": (() => __vitePreload(() => import("./angular-html.js").then((n) => n.f), true ? __vite__mapDeps([0,1,2,3]) : void 0))
  },
  {
    "id": "angular-ts",
    "name": "Angular TypeScript",
    "import": (() => __vitePreload(() => import("./angular-ts.js"), true ? __vite__mapDeps([4,0,1,2,3,5]) : void 0))
  },
  {
    "id": "apache",
    "name": "Apache Conf",
    "import": (() => __vitePreload(() => import("./apache.js"), true ? [] : void 0))
  },
  {
    "id": "apex",
    "name": "Apex",
    "import": (() => __vitePreload(() => import("./apex.js"), true ? [] : void 0))
  },
  {
    "id": "apl",
    "name": "APL",
    "import": (() => __vitePreload(() => import("./apl.js"), true ? __vite__mapDeps([6,1,2,3,7,8,9]) : void 0))
  },
  {
    "id": "applescript",
    "name": "AppleScript",
    "import": (() => __vitePreload(() => import("./applescript.js"), true ? [] : void 0))
  },
  {
    "id": "ara",
    "name": "Ara",
    "import": (() => __vitePreload(() => import("./ara.js"), true ? [] : void 0))
  },
  {
    "id": "asciidoc",
    "name": "AsciiDoc",
    "aliases": [
      "adoc"
    ],
    "import": (() => __vitePreload(() => import("./asciidoc.js"), true ? [] : void 0))
  },
  {
    "id": "asm",
    "name": "Assembly",
    "import": (() => __vitePreload(() => import("./asm.js"), true ? [] : void 0))
  },
  {
    "id": "astro",
    "name": "Astro",
    "import": (() => __vitePreload(() => import("./astro.js"), true ? __vite__mapDeps([10,9,2,11,3,12,13]) : void 0))
  },
  {
    "id": "awk",
    "name": "AWK",
    "import": (() => __vitePreload(() => import("./awk.js"), true ? [] : void 0))
  },
  {
    "id": "ballerina",
    "name": "Ballerina",
    "import": (() => __vitePreload(() => import("./ballerina.js"), true ? [] : void 0))
  },
  {
    "id": "bat",
    "name": "Batch File",
    "aliases": [
      "batch"
    ],
    "import": (() => __vitePreload(() => import("./bat.js"), true ? [] : void 0))
  },
  {
    "id": "beancount",
    "name": "Beancount",
    "import": (() => __vitePreload(() => import("./beancount.js"), true ? [] : void 0))
  },
  {
    "id": "berry",
    "name": "Berry",
    "aliases": [
      "be"
    ],
    "import": (() => __vitePreload(() => import("./berry.js"), true ? [] : void 0))
  },
  {
    "id": "bibtex",
    "name": "BibTeX",
    "import": (() => __vitePreload(() => import("./bibtex.js"), true ? [] : void 0))
  },
  {
    "id": "bicep",
    "name": "Bicep",
    "import": (() => __vitePreload(() => import("./bicep.js"), true ? [] : void 0))
  },
  {
    "id": "bird2",
    "name": "BIRD2 Configuration",
    "aliases": [
      "bird"
    ],
    "import": (() => __vitePreload(() => import("./bird2.js"), true ? [] : void 0))
  },
  {
    "id": "blade",
    "name": "Blade",
    "import": (() => __vitePreload(() => import("./blade.js"), true ? __vite__mapDeps([14,15,1,2,3,7,8,16,9]) : void 0))
  },
  {
    "id": "bsl",
    "name": "1C (Enterprise)",
    "aliases": [
      "1c"
    ],
    "import": (() => __vitePreload(() => import("./bsl.js"), true ? __vite__mapDeps([17,18]) : void 0))
  },
  {
    "id": "c",
    "name": "C",
    "import": (() => __vitePreload(() => import("./c.js"), true ? [] : void 0))
  },
  {
    "id": "c3",
    "name": "C3",
    "import": (() => __vitePreload(() => import("./c3.js"), true ? [] : void 0))
  },
  {
    "id": "cadence",
    "name": "Cadence",
    "aliases": [
      "cdc"
    ],
    "import": (() => __vitePreload(() => import("./cadence.js"), true ? [] : void 0))
  },
  {
    "id": "cairo",
    "name": "Cairo",
    "import": (() => __vitePreload(() => import("./cairo.js"), true ? __vite__mapDeps([19,20]) : void 0))
  },
  {
    "id": "clarity",
    "name": "Clarity",
    "import": (() => __vitePreload(() => import("./clarity.js"), true ? [] : void 0))
  },
  {
    "id": "clojure",
    "name": "Clojure",
    "aliases": [
      "clj"
    ],
    "import": (() => __vitePreload(() => import("./clojure.js"), true ? [] : void 0))
  },
  {
    "id": "cmake",
    "name": "CMake",
    "import": (() => __vitePreload(() => import("./cmake.js"), true ? [] : void 0))
  },
  {
    "id": "cobol",
    "name": "COBOL",
    "import": (() => __vitePreload(() => import("./cobol.js"), true ? __vite__mapDeps([21,1,2,3,8]) : void 0))
  },
  {
    "id": "codeowners",
    "name": "CODEOWNERS",
    "import": (() => __vitePreload(() => import("./codeowners.js"), true ? [] : void 0))
  },
  {
    "id": "codeql",
    "name": "CodeQL",
    "aliases": [
      "ql"
    ],
    "import": (() => __vitePreload(() => import("./codeql.js"), true ? [] : void 0))
  },
  {
    "id": "coffee",
    "name": "CoffeeScript",
    "aliases": [
      "coffeescript"
    ],
    "import": (() => __vitePreload(() => import("./coffee.js"), true ? __vite__mapDeps([22,2]) : void 0))
  },
  {
    "id": "common-lisp",
    "name": "Common Lisp",
    "aliases": [
      "lisp"
    ],
    "import": (() => __vitePreload(() => import("./common-lisp.js"), true ? [] : void 0))
  },
  {
    "id": "coq",
    "name": "Coq",
    "import": (() => __vitePreload(() => import("./coq.js"), true ? [] : void 0))
  },
  {
    "id": "cpp",
    "name": "C++",
    "aliases": [
      "c++"
    ],
    "import": (() => __vitePreload(() => import("./cpp.js"), true ? __vite__mapDeps([23,24,25,26,16]) : void 0))
  },
  {
    "id": "crystal",
    "name": "Crystal",
    "import": (() => __vitePreload(() => import("./crystal.js"), true ? __vite__mapDeps([27,1,2,3,16,26,28]) : void 0))
  },
  {
    "id": "csharp",
    "name": "C#",
    "aliases": [
      "c#",
      "cs"
    ],
    "import": (() => __vitePreload(() => import("./csharp.js"), true ? [] : void 0))
  },
  {
    "id": "css",
    "name": "CSS",
    "import": (() => __vitePreload(() => import("./css.js"), true ? [] : void 0))
  },
  {
    "id": "csv",
    "name": "CSV",
    "import": (() => __vitePreload(() => import("./csv.js"), true ? [] : void 0))
  },
  {
    "id": "cue",
    "name": "CUE",
    "import": (() => __vitePreload(() => import("./cue.js"), true ? [] : void 0))
  },
  {
    "id": "cypher",
    "name": "Cypher",
    "aliases": [
      "cql"
    ],
    "import": (() => __vitePreload(() => import("./cypher.js"), true ? [] : void 0))
  },
  {
    "id": "d",
    "name": "D",
    "import": (() => __vitePreload(() => import("./d.js"), true ? [] : void 0))
  },
  {
    "id": "dart",
    "name": "Dart",
    "import": (() => __vitePreload(() => import("./dart.js"), true ? [] : void 0))
  },
  {
    "id": "dax",
    "name": "DAX",
    "import": (() => __vitePreload(() => import("./dax.js"), true ? [] : void 0))
  },
  {
    "id": "desktop",
    "name": "Desktop",
    "import": (() => __vitePreload(() => import("./desktop.js"), true ? [] : void 0))
  },
  {
    "id": "diff",
    "name": "Diff",
    "import": (() => __vitePreload(() => import("./diff.js"), true ? [] : void 0))
  },
  {
    "id": "docker",
    "name": "Dockerfile",
    "aliases": [
      "dockerfile"
    ],
    "import": (() => __vitePreload(() => import("./docker.js"), true ? [] : void 0))
  },
  {
    "id": "dotenv",
    "name": "dotEnv",
    "import": (() => __vitePreload(() => import("./dotenv.js"), true ? [] : void 0))
  },
  {
    "id": "dream-maker",
    "name": "Dream Maker",
    "import": (() => __vitePreload(() => import("./dream-maker.js"), true ? [] : void 0))
  },
  {
    "id": "edge",
    "name": "Edge",
    "import": (() => __vitePreload(() => import("./edge.js"), true ? __vite__mapDeps([29,11,1,2,3,15]) : void 0))
  },
  {
    "id": "elixir",
    "name": "Elixir",
    "import": (() => __vitePreload(() => import("./elixir.js"), true ? __vite__mapDeps([30,1,2,3]) : void 0))
  },
  {
    "id": "elm",
    "name": "Elm",
    "import": (() => __vitePreload(() => import("./elm.js"), true ? __vite__mapDeps([31,25,26]) : void 0))
  },
  {
    "id": "emacs-lisp",
    "name": "Emacs Lisp",
    "aliases": [
      "elisp"
    ],
    "import": (() => __vitePreload(() => import("./emacs-lisp.js"), true ? [] : void 0))
  },
  {
    "id": "erb",
    "name": "ERB",
    "import": (() => __vitePreload(() => import("./erb.js"), true ? __vite__mapDeps([32,1,2,3,33,34,7,8,16,35,11,36,13,23,24,25,26,28,37,38]) : void 0))
  },
  {
    "id": "erlang",
    "name": "Erlang",
    "aliases": [
      "erl"
    ],
    "import": (() => __vitePreload(() => import("./erlang.js"), true ? __vite__mapDeps([39,40]) : void 0))
  },
  {
    "id": "fennel",
    "name": "Fennel",
    "import": (() => __vitePreload(() => import("./fennel.js"), true ? [] : void 0))
  },
  {
    "id": "fish",
    "name": "Fish",
    "import": (() => __vitePreload(() => import("./fish.js"), true ? [] : void 0))
  },
  {
    "id": "fluent",
    "name": "Fluent",
    "aliases": [
      "ftl"
    ],
    "import": (() => __vitePreload(() => import("./fluent.js"), true ? [] : void 0))
  },
  {
    "id": "fortran-fixed-form",
    "name": "Fortran (Fixed Form)",
    "aliases": [
      "f",
      "for",
      "f77"
    ],
    "import": (() => __vitePreload(() => import("./fortran-fixed-form.js"), true ? __vite__mapDeps([41,42]) : void 0))
  },
  {
    "id": "fortran-free-form",
    "name": "Fortran (Free Form)",
    "aliases": [
      "f90",
      "f95",
      "f03",
      "f08",
      "f18"
    ],
    "import": (() => __vitePreload(() => import("./fortran-free-form.js"), true ? [] : void 0))
  },
  {
    "id": "fsharp",
    "name": "F#",
    "aliases": [
      "f#",
      "fs"
    ],
    "import": (() => __vitePreload(() => import("./fsharp.js"), true ? __vite__mapDeps([43,40]) : void 0))
  },
  {
    "id": "gdresource",
    "name": "GDResource",
    "aliases": [
      "tscn",
      "tres"
    ],
    "import": (() => __vitePreload(() => import("./gdresource.js"), true ? __vite__mapDeps([44,45,46]) : void 0))
  },
  {
    "id": "gdscript",
    "name": "GDScript",
    "aliases": [
      "gd"
    ],
    "import": (() => __vitePreload(() => import("./gdscript.js"), true ? [] : void 0))
  },
  {
    "id": "gdshader",
    "name": "GDShader",
    "import": (() => __vitePreload(() => import("./gdshader.js"), true ? [] : void 0))
  },
  {
    "id": "genie",
    "name": "Genie",
    "import": (() => __vitePreload(() => import("./genie.js"), true ? [] : void 0))
  },
  {
    "id": "gherkin",
    "name": "Gherkin",
    "import": (() => __vitePreload(() => import("./gherkin.js"), true ? [] : void 0))
  },
  {
    "id": "git-commit",
    "name": "Git Commit Message",
    "import": (() => __vitePreload(() => import("./git-commit.js"), true ? __vite__mapDeps([47,48]) : void 0))
  },
  {
    "id": "git-rebase",
    "name": "Git Rebase Message",
    "import": (() => __vitePreload(() => import("./git-rebase.js"), true ? __vite__mapDeps([49,28]) : void 0))
  },
  {
    "id": "gleam",
    "name": "Gleam",
    "import": (() => __vitePreload(() => import("./gleam.js"), true ? [] : void 0))
  },
  {
    "id": "glimmer-js",
    "name": "Glimmer JS",
    "aliases": [
      "gjs"
    ],
    "import": (() => __vitePreload(() => import("./glimmer-js.js"), true ? __vite__mapDeps([50,2,11,3,1]) : void 0))
  },
  {
    "id": "glimmer-ts",
    "name": "Glimmer TS",
    "aliases": [
      "gts"
    ],
    "import": (() => __vitePreload(() => import("./glimmer-ts.js"), true ? __vite__mapDeps([51,11,3,2,1]) : void 0))
  },
  {
    "id": "glsl",
    "name": "GLSL",
    "import": (() => __vitePreload(() => import("./glsl.js"), true ? __vite__mapDeps([25,26]) : void 0))
  },
  {
    "id": "gn",
    "name": "GN",
    "import": (() => __vitePreload(() => import("./gn.js"), true ? [] : void 0))
  },
  {
    "id": "gnuplot",
    "name": "Gnuplot",
    "import": (() => __vitePreload(() => import("./gnuplot.js"), true ? [] : void 0))
  },
  {
    "id": "go",
    "name": "Go",
    "import": (() => __vitePreload(() => import("./go.js"), true ? [] : void 0))
  },
  {
    "id": "graphql",
    "name": "GraphQL",
    "aliases": [
      "gql"
    ],
    "import": (() => __vitePreload(() => import("./graphql.js"), true ? __vite__mapDeps([35,2,11,36,13]) : void 0))
  },
  {
    "id": "groovy",
    "name": "Groovy",
    "import": (() => __vitePreload(() => import("./groovy.js"), true ? [] : void 0))
  },
  {
    "id": "hack",
    "name": "Hack",
    "import": (() => __vitePreload(() => import("./hack.js"), true ? __vite__mapDeps([52,1,2,3,16]) : void 0))
  },
  {
    "id": "haml",
    "name": "Ruby Haml",
    "import": (() => __vitePreload(() => import("./haml.js"), true ? __vite__mapDeps([34,2,3]) : void 0))
  },
  {
    "id": "handlebars",
    "name": "Handlebars",
    "aliases": [
      "hbs"
    ],
    "import": (() => __vitePreload(() => import("./handlebars.js"), true ? __vite__mapDeps([53,1,2,3,38]) : void 0))
  },
  {
    "id": "haskell",
    "name": "Haskell",
    "aliases": [
      "hs"
    ],
    "import": (() => __vitePreload(() => import("./haskell.js"), true ? [] : void 0))
  },
  {
    "id": "haxe",
    "name": "Haxe",
    "import": (() => __vitePreload(() => import("./haxe.js"), true ? [] : void 0))
  },
  {
    "id": "hcl",
    "name": "HashiCorp HCL",
    "import": (() => __vitePreload(() => import("./hcl.js"), true ? [] : void 0))
  },
  {
    "id": "hjson",
    "name": "Hjson",
    "import": (() => __vitePreload(() => import("./hjson.js"), true ? [] : void 0))
  },
  {
    "id": "hlsl",
    "name": "HLSL",
    "import": (() => __vitePreload(() => import("./hlsl.js"), true ? [] : void 0))
  },
  {
    "id": "html",
    "name": "HTML",
    "import": (() => __vitePreload(() => import("./html.js"), true ? __vite__mapDeps([1,2,3]) : void 0))
  },
  {
    "id": "html-derivative",
    "name": "HTML (Derivative)",
    "import": (() => __vitePreload(() => import("./html-derivative.js"), true ? __vite__mapDeps([15,1,2,3]) : void 0))
  },
  {
    "id": "http",
    "name": "HTTP",
    "import": (() => __vitePreload(() => import("./http.js"), true ? __vite__mapDeps([54,28,9,7,8,35,2,11,36,13]) : void 0))
  },
  {
    "id": "hurl",
    "name": "Hurl",
    "import": (() => __vitePreload(() => import("./hurl.js"), true ? __vite__mapDeps([55,35,2,11,36,13,7,8,56]) : void 0))
  },
  {
    "id": "hxml",
    "name": "HXML",
    "import": (() => __vitePreload(() => import("./hxml.js"), true ? __vite__mapDeps([57,58]) : void 0))
  },
  {
    "id": "hy",
    "name": "Hy",
    "import": (() => __vitePreload(() => import("./hy.js"), true ? [] : void 0))
  },
  {
    "id": "imba",
    "name": "Imba",
    "import": (() => __vitePreload(() => import("./imba.js"), true ? [] : void 0))
  },
  {
    "id": "ini",
    "name": "INI",
    "aliases": [
      "properties"
    ],
    "import": (() => __vitePreload(() => import("./ini.js"), true ? [] : void 0))
  },
  {
    "id": "java",
    "name": "Java",
    "import": (() => __vitePreload(() => import("./java.js"), true ? [] : void 0))
  },
  {
    "id": "javascript",
    "name": "JavaScript",
    "aliases": [
      "js",
      "cjs",
      "mjs"
    ],
    "import": (() => __vitePreload(() => import("./javascript.js"), true ? [] : void 0))
  },
  {
    "id": "jinja",
    "name": "Jinja",
    "import": (() => __vitePreload(() => import("./jinja.js"), true ? __vite__mapDeps([59,1,2,3]) : void 0))
  },
  {
    "id": "jison",
    "name": "Jison",
    "import": (() => __vitePreload(() => import("./jison.js"), true ? __vite__mapDeps([60,2]) : void 0))
  },
  {
    "id": "json",
    "name": "JSON",
    "import": (() => __vitePreload(() => import("./json.js"), true ? [] : void 0))
  },
  {
    "id": "json5",
    "name": "JSON5",
    "import": (() => __vitePreload(() => import("./json5.js"), true ? [] : void 0))
  },
  {
    "id": "jsonc",
    "name": "JSON with Comments",
    "import": (() => __vitePreload(() => import("./jsonc.js"), true ? [] : void 0))
  },
  {
    "id": "jsonl",
    "name": "JSON Lines",
    "import": (() => __vitePreload(() => import("./jsonl.js"), true ? [] : void 0))
  },
  {
    "id": "jsonnet",
    "name": "Jsonnet",
    "import": (() => __vitePreload(() => import("./jsonnet.js"), true ? [] : void 0))
  },
  {
    "id": "jssm",
    "name": "JSSM",
    "aliases": [
      "fsl"
    ],
    "import": (() => __vitePreload(() => import("./jssm.js"), true ? [] : void 0))
  },
  {
    "id": "jsx",
    "name": "JSX",
    "import": (() => __vitePreload(() => import("./jsx.js"), true ? [] : void 0))
  },
  {
    "id": "julia",
    "name": "Julia",
    "aliases": [
      "jl"
    ],
    "import": (() => __vitePreload(() => import("./julia.js"), true ? __vite__mapDeps([61,23,24,25,26,16,20,2,62]) : void 0))
  },
  {
    "id": "just",
    "name": "Just",
    "import": (() => __vitePreload(() => import("./just.js"), true ? __vite__mapDeps([63,28,2,11,64,1,3,7,8,16,20,33,34,35,36,13,23,24,25,26,37,38]) : void 0))
  },
  {
    "id": "kdl",
    "name": "KDL",
    "import": (() => __vitePreload(() => import("./kdl.js"), true ? [] : void 0))
  },
  {
    "id": "kotlin",
    "name": "Kotlin",
    "aliases": [
      "kt",
      "kts"
    ],
    "import": (() => __vitePreload(() => import("./kotlin.js"), true ? [] : void 0))
  },
  {
    "id": "kusto",
    "name": "Kusto",
    "aliases": [
      "kql"
    ],
    "import": (() => __vitePreload(() => import("./kusto.js"), true ? [] : void 0))
  },
  {
    "id": "latex",
    "name": "LaTeX",
    "import": (() => __vitePreload(() => import("./latex.js"), true ? __vite__mapDeps([65,66,62]) : void 0))
  },
  {
    "id": "lean",
    "name": "Lean 4",
    "aliases": [
      "lean4"
    ],
    "import": (() => __vitePreload(() => import("./lean.js"), true ? [] : void 0))
  },
  {
    "id": "less",
    "name": "Less",
    "import": (() => __vitePreload(() => import("./less.js"), true ? [] : void 0))
  },
  {
    "id": "liquid",
    "name": "Liquid",
    "import": (() => __vitePreload(() => import("./liquid.js"), true ? __vite__mapDeps([67,1,2,3,9]) : void 0))
  },
  {
    "id": "llvm",
    "name": "LLVM IR",
    "import": (() => __vitePreload(() => import("./llvm.js"), true ? [] : void 0))
  },
  {
    "id": "log",
    "name": "Log file",
    "import": (() => __vitePreload(() => import("./log.js"), true ? [] : void 0))
  },
  {
    "id": "logo",
    "name": "Logo",
    "import": (() => __vitePreload(() => import("./logo.js"), true ? [] : void 0))
  },
  {
    "id": "lua",
    "name": "Lua",
    "import": (() => __vitePreload(() => import("./lua.js"), true ? __vite__mapDeps([37,26]) : void 0))
  },
  {
    "id": "luau",
    "name": "Luau",
    "import": (() => __vitePreload(() => import("./luau.js"), true ? [] : void 0))
  },
  {
    "id": "make",
    "name": "Makefile",
    "aliases": [
      "makefile"
    ],
    "import": (() => __vitePreload(() => import("./make.js"), true ? [] : void 0))
  },
  {
    "id": "markdown",
    "name": "Markdown",
    "aliases": [
      "md"
    ],
    "import": (() => __vitePreload(() => import("./markdown.js"), true ? [] : void 0))
  },
  {
    "id": "marko",
    "name": "Marko",
    "import": (() => __vitePreload(() => import("./marko.js"), true ? __vite__mapDeps([68,3,69,5,11]) : void 0))
  },
  {
    "id": "matlab",
    "name": "MATLAB",
    "import": (() => __vitePreload(() => import("./matlab.js"), true ? [] : void 0))
  },
  {
    "id": "mdc",
    "name": "MDC",
    "import": (() => __vitePreload(() => import("./mdc.js"), true ? __vite__mapDeps([70,40,38,15,1,2,3]) : void 0))
  },
  {
    "id": "mdx",
    "name": "MDX",
    "import": (() => __vitePreload(() => import("./mdx.js"), true ? [] : void 0))
  },
  {
    "id": "mermaid",
    "name": "Mermaid",
    "aliases": [
      "mmd"
    ],
    "import": (() => __vitePreload(() => import("./mermaid.js"), true ? [] : void 0))
  },
  {
    "id": "mipsasm",
    "name": "MIPS Assembly",
    "aliases": [
      "mips"
    ],
    "import": (() => __vitePreload(() => import("./mipsasm.js"), true ? [] : void 0))
  },
  {
    "id": "mojo",
    "name": "Mojo",
    "import": (() => __vitePreload(() => import("./mojo.js"), true ? [] : void 0))
  },
  {
    "id": "moonbit",
    "name": "MoonBit",
    "aliases": [
      "mbt",
      "mbti"
    ],
    "import": (() => __vitePreload(() => import("./moonbit.js"), true ? [] : void 0))
  },
  {
    "id": "move",
    "name": "Move",
    "import": (() => __vitePreload(() => import("./move.js"), true ? [] : void 0))
  },
  {
    "id": "narrat",
    "name": "Narrat Language",
    "aliases": [
      "nar"
    ],
    "import": (() => __vitePreload(() => import("./narrat.js"), true ? [] : void 0))
  },
  {
    "id": "nextflow",
    "name": "Nextflow",
    "aliases": [
      "nf"
    ],
    "import": (() => __vitePreload(() => import("./nextflow.js"), true ? __vite__mapDeps([71,72]) : void 0))
  },
  {
    "id": "nextflow-groovy",
    "name": "nextflow-groovy",
    "import": (() => __vitePreload(() => import("./nextflow-groovy.js"), true ? [] : void 0))
  },
  {
    "id": "nginx",
    "name": "Nginx",
    "import": (() => __vitePreload(() => import("./nginx.js"), true ? __vite__mapDeps([73,37,26]) : void 0))
  },
  {
    "id": "nim",
    "name": "Nim",
    "import": (() => __vitePreload(() => import("./nim.js"), true ? __vite__mapDeps([74,26,1,2,3,7,8,25,40]) : void 0))
  },
  {
    "id": "nix",
    "name": "Nix",
    "import": (() => __vitePreload(() => import("./nix.js"), true ? [] : void 0))
  },
  {
    "id": "nushell",
    "name": "nushell",
    "aliases": [
      "nu"
    ],
    "import": (() => __vitePreload(() => import("./nushell.js"), true ? [] : void 0))
  },
  {
    "id": "objective-c",
    "name": "Objective-C",
    "aliases": [
      "objc"
    ],
    "import": (() => __vitePreload(() => import("./objective-c.js"), true ? [] : void 0))
  },
  {
    "id": "objective-cpp",
    "name": "Objective-C++",
    "import": (() => __vitePreload(() => import("./objective-cpp.js"), true ? [] : void 0))
  },
  {
    "id": "ocaml",
    "name": "OCaml",
    "import": (() => __vitePreload(() => import("./ocaml.js"), true ? [] : void 0))
  },
  {
    "id": "odin",
    "name": "Odin",
    "import": (() => __vitePreload(() => import("./odin.js"), true ? [] : void 0))
  },
  {
    "id": "openscad",
    "name": "OpenSCAD",
    "aliases": [
      "scad"
    ],
    "import": (() => __vitePreload(() => import("./openscad.js"), true ? [] : void 0))
  },
  {
    "id": "pascal",
    "name": "Pascal",
    "import": (() => __vitePreload(() => import("./pascal.js"), true ? [] : void 0))
  },
  {
    "id": "perl",
    "name": "Perl",
    "import": (() => __vitePreload(() => import("./perl.js"), true ? __vite__mapDeps([64,1,2,3,7,8,16]) : void 0))
  },
  {
    "id": "php",
    "name": "PHP",
    "import": (() => __vitePreload(() => import("./php.js"), true ? __vite__mapDeps([75,1,2,3,7,8,16,9]) : void 0))
  },
  {
    "id": "pkl",
    "name": "Pkl",
    "import": (() => __vitePreload(() => import("./pkl.js"), true ? [] : void 0))
  },
  {
    "id": "plsql",
    "name": "PL/SQL",
    "import": (() => __vitePreload(() => import("./plsql.js"), true ? [] : void 0))
  },
  {
    "id": "po",
    "name": "Gettext PO",
    "aliases": [
      "pot",
      "potx"
    ],
    "import": (() => __vitePreload(() => import("./po.js"), true ? [] : void 0))
  },
  {
    "id": "polar",
    "name": "Polar",
    "import": (() => __vitePreload(() => import("./polar.js"), true ? [] : void 0))
  },
  {
    "id": "postcss",
    "name": "PostCSS",
    "import": (() => __vitePreload(() => import("./postcss.js"), true ? [] : void 0))
  },
  {
    "id": "powerquery",
    "name": "PowerQuery",
    "import": (() => __vitePreload(() => import("./powerquery.js"), true ? [] : void 0))
  },
  {
    "id": "powershell",
    "name": "PowerShell",
    "aliases": [
      "ps",
      "ps1"
    ],
    "import": (() => __vitePreload(() => import("./powershell.js"), true ? [] : void 0))
  },
  {
    "id": "prisma",
    "name": "Prisma",
    "import": (() => __vitePreload(() => import("./prisma.js"), true ? [] : void 0))
  },
  {
    "id": "prolog",
    "name": "Prolog",
    "import": (() => __vitePreload(() => import("./prolog.js"), true ? [] : void 0))
  },
  {
    "id": "proto",
    "name": "Protocol Buffer 3",
    "aliases": [
      "protobuf"
    ],
    "import": (() => __vitePreload(() => import("./proto.js"), true ? [] : void 0))
  },
  {
    "id": "pug",
    "name": "Pug",
    "aliases": [
      "jade"
    ],
    "import": (() => __vitePreload(() => import("./pug.js"), true ? __vite__mapDeps([76,2,3,1]) : void 0))
  },
  {
    "id": "puppet",
    "name": "Puppet",
    "import": (() => __vitePreload(() => import("./puppet.js"), true ? [] : void 0))
  },
  {
    "id": "purescript",
    "name": "PureScript",
    "import": (() => __vitePreload(() => import("./purescript.js"), true ? [] : void 0))
  },
  {
    "id": "python",
    "name": "Python",
    "aliases": [
      "py"
    ],
    "import": (() => __vitePreload(() => import("./python.js"), true ? [] : void 0))
  },
  {
    "id": "qml",
    "name": "QML",
    "import": (() => __vitePreload(() => import("./qml.js"), true ? __vite__mapDeps([77,2]) : void 0))
  },
  {
    "id": "qmldir",
    "name": "QML Directory",
    "import": (() => __vitePreload(() => import("./qmldir.js"), true ? [] : void 0))
  },
  {
    "id": "qss",
    "name": "Qt Style Sheets",
    "import": (() => __vitePreload(() => import("./qss.js"), true ? [] : void 0))
  },
  {
    "id": "r",
    "name": "R",
    "import": (() => __vitePreload(() => import("./r.js"), true ? [] : void 0))
  },
  {
    "id": "racket",
    "name": "Racket",
    "import": (() => __vitePreload(() => import("./racket.js"), true ? [] : void 0))
  },
  {
    "id": "raku",
    "name": "Raku",
    "aliases": [
      "perl6"
    ],
    "import": (() => __vitePreload(() => import("./raku.js"), true ? [] : void 0))
  },
  {
    "id": "razor",
    "name": "ASP.NET Razor",
    "import": (() => __vitePreload(() => import("./razor.js"), true ? __vite__mapDeps([78,1,2,3,79]) : void 0))
  },
  {
    "id": "reg",
    "name": "Windows Registry Script",
    "import": (() => __vitePreload(() => import("./reg.js"), true ? [] : void 0))
  },
  {
    "id": "regexp",
    "name": "RegExp",
    "aliases": [
      "regex"
    ],
    "import": (() => __vitePreload(() => import("./regexp.js"), true ? [] : void 0))
  },
  {
    "id": "rel",
    "name": "Rel",
    "import": (() => __vitePreload(() => import("./rel.js"), true ? [] : void 0))
  },
  {
    "id": "riscv",
    "name": "RISC-V",
    "import": (() => __vitePreload(() => import("./riscv.js"), true ? [] : void 0))
  },
  {
    "id": "ron",
    "name": "RON",
    "import": (() => __vitePreload(() => import("./ron.js"), true ? [] : void 0))
  },
  {
    "id": "rosmsg",
    "name": "ROS Interface",
    "import": (() => __vitePreload(() => import("./rosmsg.js"), true ? [] : void 0))
  },
  {
    "id": "rst",
    "name": "reStructuredText",
    "import": (() => __vitePreload(() => import("./rst.js"), true ? __vite__mapDeps([80,15,1,2,3,23,24,25,26,16,20,28,38,81,33,34,7,8,35,11,36,13,37]) : void 0))
  },
  {
    "id": "ruby",
    "name": "Ruby",
    "aliases": [
      "rb"
    ],
    "import": (() => __vitePreload(() => import("./ruby.js"), true ? __vite__mapDeps([33,1,2,3,34,7,8,16,35,11,36,13,23,24,25,26,28,37,38]) : void 0))
  },
  {
    "id": "rust",
    "name": "Rust",
    "aliases": [
      "rs"
    ],
    "import": (() => __vitePreload(() => import("./rust.js"), true ? [] : void 0))
  },
  {
    "id": "sas",
    "name": "SAS",
    "import": (() => __vitePreload(() => import("./sas.js"), true ? __vite__mapDeps([82,16]) : void 0))
  },
  {
    "id": "sass",
    "name": "Sass",
    "import": (() => __vitePreload(() => import("./sass.js"), true ? [] : void 0))
  },
  {
    "id": "scala",
    "name": "Scala",
    "import": (() => __vitePreload(() => import("./scala.js"), true ? [] : void 0))
  },
  {
    "id": "scheme",
    "name": "Scheme",
    "import": (() => __vitePreload(() => import("./scheme.js"), true ? [] : void 0))
  },
  {
    "id": "scss",
    "name": "SCSS",
    "import": (() => __vitePreload(() => import("./scss.js"), true ? __vite__mapDeps([5,3]) : void 0))
  },
  {
    "id": "sdbl",
    "name": "1C (Query)",
    "aliases": [
      "1c-query"
    ],
    "import": (() => __vitePreload(() => import("./sdbl.js"), true ? [] : void 0))
  },
  {
    "id": "shaderlab",
    "name": "ShaderLab",
    "aliases": [
      "shader"
    ],
    "import": (() => __vitePreload(() => import("./shaderlab.js"), true ? __vite__mapDeps([83,84]) : void 0))
  },
  {
    "id": "shellscript",
    "name": "Shell",
    "aliases": [
      "bash",
      "sh",
      "shell",
      "zsh"
    ],
    "import": (() => __vitePreload(() => import("./shellscript.js"), true ? [] : void 0))
  },
  {
    "id": "shellsession",
    "name": "Shell Session",
    "aliases": [
      "console"
    ],
    "import": (() => __vitePreload(() => import("./shellsession.js"), true ? __vite__mapDeps([85,28]) : void 0))
  },
  {
    "id": "smalltalk",
    "name": "Smalltalk",
    "import": (() => __vitePreload(() => import("./smalltalk.js"), true ? [] : void 0))
  },
  {
    "id": "solidity",
    "name": "Solidity",
    "import": (() => __vitePreload(() => import("./solidity.js"), true ? [] : void 0))
  },
  {
    "id": "soy",
    "name": "Closure Templates",
    "aliases": [
      "closure-templates"
    ],
    "import": (() => __vitePreload(() => import("./soy.js"), true ? __vite__mapDeps([86,1,2,3]) : void 0))
  },
  {
    "id": "sparql",
    "name": "SPARQL",
    "import": (() => __vitePreload(() => import("./sparql.js"), true ? __vite__mapDeps([87,88]) : void 0))
  },
  {
    "id": "splunk",
    "name": "Splunk Query Language",
    "aliases": [
      "spl"
    ],
    "import": (() => __vitePreload(() => import("./splunk.js"), true ? [] : void 0))
  },
  {
    "id": "sql",
    "name": "SQL",
    "import": (() => __vitePreload(() => import("./sql.js"), true ? [] : void 0))
  },
  {
    "id": "ssh-config",
    "name": "SSH Config",
    "import": (() => __vitePreload(() => import("./ssh-config.js"), true ? [] : void 0))
  },
  {
    "id": "stata",
    "name": "Stata",
    "import": (() => __vitePreload(() => import("./stata.js"), true ? __vite__mapDeps([89,16]) : void 0))
  },
  {
    "id": "stylus",
    "name": "Stylus",
    "aliases": [
      "styl"
    ],
    "import": (() => __vitePreload(() => import("./stylus.js"), true ? [] : void 0))
  },
  {
    "id": "surrealql",
    "name": "SurrealQL",
    "aliases": [
      "surql"
    ],
    "import": (() => __vitePreload(() => import("./surrealql.js"), true ? __vite__mapDeps([90,2]) : void 0))
  },
  {
    "id": "svelte",
    "name": "Svelte",
    "import": (() => __vitePreload(() => import("./svelte.js"), true ? __vite__mapDeps([91,2,11,3,12]) : void 0))
  },
  {
    "id": "swift",
    "name": "Swift",
    "import": (() => __vitePreload(() => import("./swift.js"), true ? [] : void 0))
  },
  {
    "id": "system-verilog",
    "name": "SystemVerilog",
    "import": (() => __vitePreload(() => import("./system-verilog.js"), true ? [] : void 0))
  },
  {
    "id": "systemd",
    "name": "Systemd Units",
    "import": (() => __vitePreload(() => import("./systemd.js"), true ? [] : void 0))
  },
  {
    "id": "talonscript",
    "name": "TalonScript",
    "aliases": [
      "talon"
    ],
    "import": (() => __vitePreload(() => import("./talonscript.js"), true ? [] : void 0))
  },
  {
    "id": "tasl",
    "name": "Tasl",
    "import": (() => __vitePreload(() => import("./tasl.js"), true ? [] : void 0))
  },
  {
    "id": "tcl",
    "name": "Tcl",
    "import": (() => __vitePreload(() => import("./tcl.js"), true ? [] : void 0))
  },
  {
    "id": "templ",
    "name": "Templ",
    "import": (() => __vitePreload(() => import("./templ.js"), true ? __vite__mapDeps([92,93,2,3]) : void 0))
  },
  {
    "id": "terraform",
    "name": "Terraform",
    "aliases": [
      "tf",
      "tfvars"
    ],
    "import": (() => __vitePreload(() => import("./terraform.js"), true ? [] : void 0))
  },
  {
    "id": "tex",
    "name": "TeX",
    "import": (() => __vitePreload(() => import("./tex.js"), true ? __vite__mapDeps([66,62]) : void 0))
  },
  {
    "id": "toml",
    "name": "TOML",
    "import": (() => __vitePreload(() => import("./toml.js"), true ? [] : void 0))
  },
  {
    "id": "ts-tags",
    "name": "TypeScript with Tags",
    "aliases": [
      "lit"
    ],
    "import": (() => __vitePreload(() => import("./ts-tags.js"), true ? __vite__mapDeps([94,11,3,2,25,26,1,16,7,8]) : void 0))
  },
  {
    "id": "tsv",
    "name": "TSV",
    "import": (() => __vitePreload(() => import("./tsv.js"), true ? [] : void 0))
  },
  {
    "id": "tsx",
    "name": "TSX",
    "import": (() => __vitePreload(() => import("./tsx.js"), true ? [] : void 0))
  },
  {
    "id": "turtle",
    "name": "Turtle",
    "import": (() => __vitePreload(() => import("./turtle.js"), true ? [] : void 0))
  },
  {
    "id": "twig",
    "name": "Twig",
    "import": (() => __vitePreload(() => import("./twig.js"), true ? __vite__mapDeps([95,3,2,5,75,1,7,8,16,9,20,33,34,35,11,36,13,23,24,25,26,28,37,38]) : void 0))
  },
  {
    "id": "typescript",
    "name": "TypeScript",
    "aliases": [
      "ts",
      "cts",
      "mts"
    ],
    "import": (() => __vitePreload(() => import("./typescript.js"), true ? [] : void 0))
  },
  {
    "id": "typespec",
    "name": "TypeSpec",
    "aliases": [
      "tsp"
    ],
    "import": (() => __vitePreload(() => import("./typespec.js"), true ? [] : void 0))
  },
  {
    "id": "typst",
    "name": "Typst",
    "aliases": [
      "typ"
    ],
    "import": (() => __vitePreload(() => import("./typst.js"), true ? [] : void 0))
  },
  {
    "id": "v",
    "name": "V",
    "import": (() => __vitePreload(() => import("./v.js"), true ? [] : void 0))
  },
  {
    "id": "vala",
    "name": "Vala",
    "import": (() => __vitePreload(() => import("./vala.js"), true ? [] : void 0))
  },
  {
    "id": "vb",
    "name": "Visual Basic",
    "aliases": [
      "cmd"
    ],
    "import": (() => __vitePreload(() => import("./vb.js"), true ? [] : void 0))
  },
  {
    "id": "verilog",
    "name": "Verilog",
    "import": (() => __vitePreload(() => import("./verilog.js"), true ? [] : void 0))
  },
  {
    "id": "vhdl",
    "name": "VHDL",
    "import": (() => __vitePreload(() => import("./vhdl.js"), true ? [] : void 0))
  },
  {
    "id": "viml",
    "name": "Vim Script",
    "aliases": [
      "vim",
      "vimscript"
    ],
    "import": (() => __vitePreload(() => import("./viml.js"), true ? [] : void 0))
  },
  {
    "id": "vue",
    "name": "Vue",
    "import": (() => __vitePreload(() => import("./vue.js"), true ? __vite__mapDeps([96,3,2,11,9,1,15]) : void 0))
  },
  {
    "id": "vue-html",
    "name": "Vue HTML",
    "import": (() => __vitePreload(() => import("./vue-html.js"), true ? __vite__mapDeps([97,2]) : void 0))
  },
  {
    "id": "vue-vine",
    "name": "Vue Vine",
    "import": (() => __vitePreload(() => import("./vue-vine.js"), true ? __vite__mapDeps([98,3,5,69,99,12,2]) : void 0))
  },
  {
    "id": "vyper",
    "name": "Vyper",
    "aliases": [
      "vy"
    ],
    "import": (() => __vitePreload(() => import("./vyper.js"), true ? [] : void 0))
  },
  {
    "id": "wasm",
    "name": "WebAssembly",
    "import": (() => __vitePreload(() => import("./wasm.js"), true ? [] : void 0))
  },
  {
    "id": "wenyan",
    "name": "Wenyan",
    "aliases": [
      "文言"
    ],
    "import": (() => __vitePreload(() => import("./wenyan.js"), true ? [] : void 0))
  },
  {
    "id": "wgsl",
    "name": "WGSL",
    "import": (() => __vitePreload(() => import("./wgsl.js"), true ? [] : void 0))
  },
  {
    "id": "wikitext",
    "name": "Wikitext",
    "aliases": [
      "mediawiki",
      "wiki"
    ],
    "import": (() => __vitePreload(() => import("./wikitext.js"), true ? [] : void 0))
  },
  {
    "id": "wit",
    "name": "WebAssembly Interface Types",
    "import": (() => __vitePreload(() => import("./wit.js"), true ? [] : void 0))
  },
  {
    "id": "wolfram",
    "name": "Wolfram",
    "aliases": [
      "wl"
    ],
    "import": (() => __vitePreload(() => import("./wolfram.js"), true ? [] : void 0))
  },
  {
    "id": "xml",
    "name": "XML",
    "import": (() => __vitePreload(() => import("./xml.js"), true ? __vite__mapDeps([7,8]) : void 0))
  },
  {
    "id": "xsl",
    "name": "XSL",
    "import": (() => __vitePreload(() => import("./xsl.js"), true ? __vite__mapDeps([100,7,8]) : void 0))
  },
  {
    "id": "yaml",
    "name": "YAML",
    "aliases": [
      "yml"
    ],
    "import": (() => __vitePreload(() => import("./yaml.js"), true ? [] : void 0))
  },
  {
    "id": "zenscript",
    "name": "ZenScript",
    "import": (() => __vitePreload(() => import("./zenscript.js"), true ? [] : void 0))
  },
  {
    "id": "zig",
    "name": "Zig",
    "import": (() => __vitePreload(() => import("./zig.js"), true ? [] : void 0))
  }
];
const bundledLanguagesBase = Object.fromEntries(bundledLanguagesInfo.map((i2) => [i2.id, i2.import]));
const bundledLanguagesAlias = Object.fromEntries(bundledLanguagesInfo.flatMap((i2) => {
  var _a2;
  return ((_a2 = i2.aliases) == null ? void 0 : _a2.map((a) => [a, i2.import])) || [];
}));
const bundledLanguages = {
  ...bundledLanguagesBase,
  ...bundledLanguagesAlias
};
const bundledThemesInfo = [
  {
    "id": "andromeeda",
    "displayName": "Andromeeda",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./andromeeda.js"), true ? [] : void 0))
  },
  {
    "id": "aurora-x",
    "displayName": "Aurora X",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./aurora-x.js"), true ? [] : void 0))
  },
  {
    "id": "ayu-dark",
    "displayName": "Ayu Dark",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./ayu-dark.js"), true ? [] : void 0))
  },
  {
    "id": "ayu-light",
    "displayName": "Ayu Light",
    "type": "light",
    "import": (() => __vitePreload(() => import("./ayu-light.js"), true ? [] : void 0))
  },
  {
    "id": "ayu-mirage",
    "displayName": "Ayu Mirage",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./ayu-mirage.js"), true ? [] : void 0))
  },
  {
    "id": "catppuccin-frappe",
    "displayName": "Catppuccin Frappé",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./catppuccin-frappe.js"), true ? [] : void 0))
  },
  {
    "id": "catppuccin-latte",
    "displayName": "Catppuccin Latte",
    "type": "light",
    "import": (() => __vitePreload(() => import("./catppuccin-latte.js"), true ? [] : void 0))
  },
  {
    "id": "catppuccin-macchiato",
    "displayName": "Catppuccin Macchiato",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./catppuccin-macchiato.js"), true ? [] : void 0))
  },
  {
    "id": "catppuccin-mocha",
    "displayName": "Catppuccin Mocha",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./catppuccin-mocha.js"), true ? [] : void 0))
  },
  {
    "id": "dark-plus",
    "displayName": "Dark Plus",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./dark-plus.js"), true ? [] : void 0))
  },
  {
    "id": "dracula",
    "displayName": "Dracula Theme",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./dracula.js"), true ? [] : void 0))
  },
  {
    "id": "dracula-soft",
    "displayName": "Dracula Theme Soft",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./dracula-soft.js"), true ? [] : void 0))
  },
  {
    "id": "everforest-dark",
    "displayName": "Everforest Dark",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./everforest-dark.js"), true ? [] : void 0))
  },
  {
    "id": "everforest-light",
    "displayName": "Everforest Light",
    "type": "light",
    "import": (() => __vitePreload(() => import("./everforest-light.js"), true ? [] : void 0))
  },
  {
    "id": "github-dark",
    "displayName": "GitHub Dark",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./github-dark.js"), true ? [] : void 0))
  },
  {
    "id": "github-dark-default",
    "displayName": "GitHub Dark Default",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./github-dark-default.js"), true ? [] : void 0))
  },
  {
    "id": "github-dark-dimmed",
    "displayName": "GitHub Dark Dimmed",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./github-dark-dimmed.js"), true ? [] : void 0))
  },
  {
    "id": "github-dark-high-contrast",
    "displayName": "GitHub Dark High Contrast",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./github-dark-high-contrast.js"), true ? [] : void 0))
  },
  {
    "id": "github-light",
    "displayName": "GitHub Light",
    "type": "light",
    "import": (() => __vitePreload(() => import("./github-light.js"), true ? [] : void 0))
  },
  {
    "id": "github-light-default",
    "displayName": "GitHub Light Default",
    "type": "light",
    "import": (() => __vitePreload(() => import("./github-light-default.js"), true ? [] : void 0))
  },
  {
    "id": "github-light-high-contrast",
    "displayName": "GitHub Light High Contrast",
    "type": "light",
    "import": (() => __vitePreload(() => import("./github-light-high-contrast.js"), true ? [] : void 0))
  },
  {
    "id": "gruvbox-dark-hard",
    "displayName": "Gruvbox Dark Hard",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./gruvbox-dark-hard.js"), true ? [] : void 0))
  },
  {
    "id": "gruvbox-dark-medium",
    "displayName": "Gruvbox Dark Medium",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./gruvbox-dark-medium.js"), true ? [] : void 0))
  },
  {
    "id": "gruvbox-dark-soft",
    "displayName": "Gruvbox Dark Soft",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./gruvbox-dark-soft.js"), true ? [] : void 0))
  },
  {
    "id": "gruvbox-light-hard",
    "displayName": "Gruvbox Light Hard",
    "type": "light",
    "import": (() => __vitePreload(() => import("./gruvbox-light-hard.js"), true ? [] : void 0))
  },
  {
    "id": "gruvbox-light-medium",
    "displayName": "Gruvbox Light Medium",
    "type": "light",
    "import": (() => __vitePreload(() => import("./gruvbox-light-medium.js"), true ? [] : void 0))
  },
  {
    "id": "gruvbox-light-soft",
    "displayName": "Gruvbox Light Soft",
    "type": "light",
    "import": (() => __vitePreload(() => import("./gruvbox-light-soft.js"), true ? [] : void 0))
  },
  {
    "id": "horizon",
    "displayName": "Horizon",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./horizon.js"), true ? [] : void 0))
  },
  {
    "id": "horizon-bright",
    "displayName": "Horizon Bright",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./horizon-bright.js"), true ? [] : void 0))
  },
  {
    "id": "houston",
    "displayName": "Houston",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./houston.js"), true ? [] : void 0))
  },
  {
    "id": "kanagawa-dragon",
    "displayName": "Kanagawa Dragon",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./kanagawa-dragon.js"), true ? [] : void 0))
  },
  {
    "id": "kanagawa-lotus",
    "displayName": "Kanagawa Lotus",
    "type": "light",
    "import": (() => __vitePreload(() => import("./kanagawa-lotus.js"), true ? [] : void 0))
  },
  {
    "id": "kanagawa-wave",
    "displayName": "Kanagawa Wave",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./kanagawa-wave.js"), true ? [] : void 0))
  },
  {
    "id": "laserwave",
    "displayName": "LaserWave",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./laserwave.js"), true ? [] : void 0))
  },
  {
    "id": "light-plus",
    "displayName": "Light Plus",
    "type": "light",
    "import": (() => __vitePreload(() => import("./light-plus.js"), true ? [] : void 0))
  },
  {
    "id": "material-theme",
    "displayName": "Material Theme",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./material-theme.js"), true ? [] : void 0))
  },
  {
    "id": "material-theme-darker",
    "displayName": "Material Theme Darker",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./material-theme-darker.js"), true ? [] : void 0))
  },
  {
    "id": "material-theme-lighter",
    "displayName": "Material Theme Lighter",
    "type": "light",
    "import": (() => __vitePreload(() => import("./material-theme-lighter.js"), true ? [] : void 0))
  },
  {
    "id": "material-theme-ocean",
    "displayName": "Material Theme Ocean",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./material-theme-ocean.js"), true ? [] : void 0))
  },
  {
    "id": "material-theme-palenight",
    "displayName": "Material Theme Palenight",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./material-theme-palenight.js"), true ? [] : void 0))
  },
  {
    "id": "min-dark",
    "displayName": "Min Dark",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./min-dark.js"), true ? [] : void 0))
  },
  {
    "id": "min-light",
    "displayName": "Min Light",
    "type": "light",
    "import": (() => __vitePreload(() => import("./min-light.js"), true ? [] : void 0))
  },
  {
    "id": "monokai",
    "displayName": "Monokai",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./monokai.js"), true ? [] : void 0))
  },
  {
    "id": "night-owl",
    "displayName": "Night Owl",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./night-owl.js"), true ? [] : void 0))
  },
  {
    "id": "night-owl-light",
    "displayName": "Night Owl Light",
    "type": "light",
    "import": (() => __vitePreload(() => import("./night-owl-light.js"), true ? [] : void 0))
  },
  {
    "id": "nord",
    "displayName": "Nord",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./nord.js"), true ? [] : void 0))
  },
  {
    "id": "one-dark-pro",
    "displayName": "One Dark Pro",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./one-dark-pro.js"), true ? [] : void 0))
  },
  {
    "id": "one-light",
    "displayName": "One Light",
    "type": "light",
    "import": (() => __vitePreload(() => import("./one-light.js"), true ? [] : void 0))
  },
  {
    "id": "plastic",
    "displayName": "Plastic",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./plastic.js"), true ? [] : void 0))
  },
  {
    "id": "poimandres",
    "displayName": "Poimandres",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./poimandres.js"), true ? [] : void 0))
  },
  {
    "id": "red",
    "displayName": "Red",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./red.js"), true ? [] : void 0))
  },
  {
    "id": "rose-pine",
    "displayName": "Rosé Pine",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./rose-pine.js"), true ? [] : void 0))
  },
  {
    "id": "rose-pine-dawn",
    "displayName": "Rosé Pine Dawn",
    "type": "light",
    "import": (() => __vitePreload(() => import("./rose-pine-dawn.js"), true ? [] : void 0))
  },
  {
    "id": "rose-pine-moon",
    "displayName": "Rosé Pine Moon",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./rose-pine-moon.js"), true ? [] : void 0))
  },
  {
    "id": "slack-dark",
    "displayName": "Slack Dark",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./slack-dark.js"), true ? [] : void 0))
  },
  {
    "id": "slack-ochin",
    "displayName": "Slack Ochin",
    "type": "light",
    "import": (() => __vitePreload(() => import("./slack-ochin.js"), true ? [] : void 0))
  },
  {
    "id": "snazzy-light",
    "displayName": "Snazzy Light",
    "type": "light",
    "import": (() => __vitePreload(() => import("./snazzy-light.js"), true ? [] : void 0))
  },
  {
    "id": "solarized-dark",
    "displayName": "Solarized Dark",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./solarized-dark.js"), true ? [] : void 0))
  },
  {
    "id": "solarized-light",
    "displayName": "Solarized Light",
    "type": "light",
    "import": (() => __vitePreload(() => import("./solarized-light.js"), true ? [] : void 0))
  },
  {
    "id": "synthwave-84",
    "displayName": "Synthwave '84",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./synthwave-84.js"), true ? [] : void 0))
  },
  {
    "id": "tokyo-night",
    "displayName": "Tokyo Night",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./tokyo-night.js"), true ? [] : void 0))
  },
  {
    "id": "vesper",
    "displayName": "Vesper",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./vesper.js"), true ? [] : void 0))
  },
  {
    "id": "vitesse-black",
    "displayName": "Vitesse Black",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./vitesse-black.js"), true ? [] : void 0))
  },
  {
    "id": "vitesse-dark",
    "displayName": "Vitesse Dark",
    "type": "dark",
    "import": (() => __vitePreload(() => import("./vitesse-dark.js"), true ? [] : void 0))
  },
  {
    "id": "vitesse-light",
    "displayName": "Vitesse Light",
    "type": "light",
    "import": (() => __vitePreload(() => import("./vitesse-light.js"), true ? [] : void 0))
  }
];
const bundledThemes = Object.fromEntries(bundledThemesInfo.map((i2) => [i2.id, i2.import]));
class ShikiError3 extends Error {
  constructor(message) {
    super(message);
    this.name = "ShikiError";
  }
}
function getHeapMax() {
  return 2147483648;
}
function _emscripten_get_now() {
  return typeof performance !== "undefined" ? performance.now() : Date.now();
}
const alignUp = (x2, multiple) => x2 + (multiple - x2 % multiple) % multiple;
async function main(init) {
  let wasmMemory;
  let buffer;
  const binding = {};
  function updateGlobalBufferAndViews(buf) {
    buffer = buf;
    binding.HEAPU8 = new Uint8Array(buf);
    binding.HEAPU32 = new Uint32Array(buf);
  }
  function _emscripten_memcpy_big(dest, src, num) {
    binding.HEAPU8.copyWithin(dest, src, src + num);
  }
  function emscripten_realloc_buffer(size) {
    try {
      wasmMemory.grow(size - buffer.byteLength + 65535 >>> 16);
      updateGlobalBufferAndViews(wasmMemory.buffer);
      return 1;
    } catch {
    }
  }
  function _emscripten_resize_heap(requestedSize) {
    const oldSize = binding.HEAPU8.length;
    requestedSize = requestedSize >>> 0;
    const maxHeapSize = getHeapMax();
    if (requestedSize > maxHeapSize)
      return false;
    for (let cutDown = 1; cutDown <= 4; cutDown *= 2) {
      let overGrownHeapSize = oldSize * (1 + 0.2 / cutDown);
      overGrownHeapSize = Math.min(overGrownHeapSize, requestedSize + 100663296);
      const newSize = Math.min(maxHeapSize, alignUp(Math.max(requestedSize, overGrownHeapSize), 65536));
      const replacement = emscripten_realloc_buffer(newSize);
      if (replacement)
        return true;
    }
    return false;
  }
  const UTF8Decoder = typeof TextDecoder != "undefined" ? new TextDecoder("utf8") : void 0;
  function UTF8ArrayToString(heapOrArray, idx, maxBytesToRead = 1024) {
    const endIdx = idx + maxBytesToRead;
    let endPtr = idx;
    while (heapOrArray[endPtr] && !(endPtr >= endIdx)) ++endPtr;
    if (endPtr - idx > 16 && heapOrArray.buffer && UTF8Decoder) {
      return UTF8Decoder.decode(heapOrArray.subarray(idx, endPtr));
    }
    let str = "";
    while (idx < endPtr) {
      let u0 = heapOrArray[idx++];
      if (!(u0 & 128)) {
        str += String.fromCharCode(u0);
        continue;
      }
      const u1 = heapOrArray[idx++] & 63;
      if ((u0 & 224) === 192) {
        str += String.fromCharCode((u0 & 31) << 6 | u1);
        continue;
      }
      const u2 = heapOrArray[idx++] & 63;
      if ((u0 & 240) === 224) {
        u0 = (u0 & 15) << 12 | u1 << 6 | u2;
      } else {
        u0 = (u0 & 7) << 18 | u1 << 12 | u2 << 6 | heapOrArray[idx++] & 63;
      }
      if (u0 < 65536) {
        str += String.fromCharCode(u0);
      } else {
        const ch = u0 - 65536;
        str += String.fromCharCode(55296 | ch >> 10, 56320 | ch & 1023);
      }
    }
    return str;
  }
  function UTF8ToString(ptr, maxBytesToRead) {
    return ptr ? UTF8ArrayToString(binding.HEAPU8, ptr, maxBytesToRead) : "";
  }
  const asmLibraryArg = {
    emscripten_get_now: _emscripten_get_now,
    emscripten_memcpy_big: _emscripten_memcpy_big,
    emscripten_resize_heap: _emscripten_resize_heap,
    fd_write: () => 0
  };
  async function createWasm() {
    const info = {
      env: asmLibraryArg,
      wasi_snapshot_preview1: asmLibraryArg
    };
    const exports$1 = await init(info);
    wasmMemory = exports$1.memory;
    updateGlobalBufferAndViews(wasmMemory.buffer);
    Object.assign(binding, exports$1);
    binding.UTF8ToString = UTF8ToString;
  }
  await createWasm();
  return binding;
}
var __defProp2 = Object.defineProperty;
var __defNormalProp2 = (obj, key2, value) => key2 in obj ? __defProp2(obj, key2, { enumerable: true, configurable: true, writable: true, value }) : obj[key2] = value;
var __publicField2 = (obj, key2, value) => __defNormalProp2(obj, typeof key2 !== "symbol" ? key2 + "" : key2, value);
let onigBinding = null;
function throwLastOnigError(onigBinding2) {
  throw new ShikiError3(onigBinding2.UTF8ToString(onigBinding2.getLastOnigError()));
}
class UtfString {
  constructor(str) {
    __publicField2(this, "utf16Length");
    __publicField2(this, "utf8Length");
    __publicField2(this, "utf16Value");
    __publicField2(this, "utf8Value");
    __publicField2(this, "utf16OffsetToUtf8");
    __publicField2(this, "utf8OffsetToUtf16");
    const utf16Length = str.length;
    const utf8Length = UtfString._utf8ByteLength(str);
    const computeIndicesMapping = utf8Length !== utf16Length;
    const utf16OffsetToUtf8 = computeIndicesMapping ? new Uint32Array(utf16Length + 1) : null;
    if (computeIndicesMapping)
      utf16OffsetToUtf8[utf16Length] = utf8Length;
    const utf8OffsetToUtf16 = computeIndicesMapping ? new Uint32Array(utf8Length + 1) : null;
    if (computeIndicesMapping)
      utf8OffsetToUtf16[utf8Length] = utf16Length;
    const utf8Value = new Uint8Array(utf8Length);
    let i8 = 0;
    for (let i16 = 0; i16 < utf16Length; i16++) {
      const charCode = str.charCodeAt(i16);
      let codePoint = charCode;
      let wasSurrogatePair = false;
      if (charCode >= 55296 && charCode <= 56319) {
        if (i16 + 1 < utf16Length) {
          const nextCharCode = str.charCodeAt(i16 + 1);
          if (nextCharCode >= 56320 && nextCharCode <= 57343) {
            codePoint = (charCode - 55296 << 10) + 65536 | nextCharCode - 56320;
            wasSurrogatePair = true;
          }
        }
      }
      if (computeIndicesMapping) {
        utf16OffsetToUtf8[i16] = i8;
        if (wasSurrogatePair)
          utf16OffsetToUtf8[i16 + 1] = i8;
        if (codePoint <= 127) {
          utf8OffsetToUtf16[i8 + 0] = i16;
        } else if (codePoint <= 2047) {
          utf8OffsetToUtf16[i8 + 0] = i16;
          utf8OffsetToUtf16[i8 + 1] = i16;
        } else if (codePoint <= 65535) {
          utf8OffsetToUtf16[i8 + 0] = i16;
          utf8OffsetToUtf16[i8 + 1] = i16;
          utf8OffsetToUtf16[i8 + 2] = i16;
        } else {
          utf8OffsetToUtf16[i8 + 0] = i16;
          utf8OffsetToUtf16[i8 + 1] = i16;
          utf8OffsetToUtf16[i8 + 2] = i16;
          utf8OffsetToUtf16[i8 + 3] = i16;
        }
      }
      if (codePoint <= 127) {
        utf8Value[i8++] = codePoint;
      } else if (codePoint <= 2047) {
        utf8Value[i8++] = 192 | (codePoint & 1984) >>> 6;
        utf8Value[i8++] = 128 | (codePoint & 63) >>> 0;
      } else if (codePoint <= 65535) {
        utf8Value[i8++] = 224 | (codePoint & 61440) >>> 12;
        utf8Value[i8++] = 128 | (codePoint & 4032) >>> 6;
        utf8Value[i8++] = 128 | (codePoint & 63) >>> 0;
      } else {
        utf8Value[i8++] = 240 | (codePoint & 1835008) >>> 18;
        utf8Value[i8++] = 128 | (codePoint & 258048) >>> 12;
        utf8Value[i8++] = 128 | (codePoint & 4032) >>> 6;
        utf8Value[i8++] = 128 | (codePoint & 63) >>> 0;
      }
      if (wasSurrogatePair)
        i16++;
    }
    this.utf16Length = utf16Length;
    this.utf8Length = utf8Length;
    this.utf16Value = str;
    this.utf8Value = utf8Value;
    this.utf16OffsetToUtf8 = utf16OffsetToUtf8;
    this.utf8OffsetToUtf16 = utf8OffsetToUtf16;
  }
  static _utf8ByteLength(str) {
    let result = 0;
    for (let i2 = 0, len = str.length; i2 < len; i2++) {
      const charCode = str.charCodeAt(i2);
      let codepoint = charCode;
      let wasSurrogatePair = false;
      if (charCode >= 55296 && charCode <= 56319) {
        if (i2 + 1 < len) {
          const nextCharCode = str.charCodeAt(i2 + 1);
          if (nextCharCode >= 56320 && nextCharCode <= 57343) {
            codepoint = (charCode - 55296 << 10) + 65536 | nextCharCode - 56320;
            wasSurrogatePair = true;
          }
        }
      }
      if (codepoint <= 127)
        result += 1;
      else if (codepoint <= 2047)
        result += 2;
      else if (codepoint <= 65535)
        result += 3;
      else
        result += 4;
      if (wasSurrogatePair)
        i2++;
    }
    return result;
  }
  createString(onigBinding2) {
    const result = onigBinding2.omalloc(this.utf8Length);
    onigBinding2.HEAPU8.set(this.utf8Value, result);
    return result;
  }
}
const _OnigString = class _OnigString2 {
  constructor(str) {
    __publicField2(this, "id", ++_OnigString2.LAST_ID);
    __publicField2(this, "_onigBinding");
    __publicField2(this, "content");
    __publicField2(this, "utf16Length");
    __publicField2(this, "utf8Length");
    __publicField2(this, "utf16OffsetToUtf8");
    __publicField2(this, "utf8OffsetToUtf16");
    __publicField2(this, "ptr");
    if (!onigBinding)
      throw new ShikiError3("Must invoke loadWasm first.");
    this._onigBinding = onigBinding;
    this.content = str;
    const utfString = new UtfString(str);
    this.utf16Length = utfString.utf16Length;
    this.utf8Length = utfString.utf8Length;
    this.utf16OffsetToUtf8 = utfString.utf16OffsetToUtf8;
    this.utf8OffsetToUtf16 = utfString.utf8OffsetToUtf16;
    if (this.utf8Length < 1e4 && !_OnigString2._sharedPtrInUse) {
      if (!_OnigString2._sharedPtr)
        _OnigString2._sharedPtr = onigBinding.omalloc(1e4);
      _OnigString2._sharedPtrInUse = true;
      onigBinding.HEAPU8.set(utfString.utf8Value, _OnigString2._sharedPtr);
      this.ptr = _OnigString2._sharedPtr;
    } else {
      this.ptr = utfString.createString(onigBinding);
    }
  }
  convertUtf8OffsetToUtf16(utf8Offset) {
    if (this.utf8OffsetToUtf16) {
      if (utf8Offset < 0)
        return 0;
      if (utf8Offset > this.utf8Length)
        return this.utf16Length;
      return this.utf8OffsetToUtf16[utf8Offset];
    }
    return utf8Offset;
  }
  convertUtf16OffsetToUtf8(utf16Offset) {
    if (this.utf16OffsetToUtf8) {
      if (utf16Offset < 0)
        return 0;
      if (utf16Offset > this.utf16Length)
        return this.utf8Length;
      return this.utf16OffsetToUtf8[utf16Offset];
    }
    return utf16Offset;
  }
  dispose() {
    if (this.ptr === _OnigString2._sharedPtr)
      _OnigString2._sharedPtrInUse = false;
    else
      this._onigBinding.ofree(this.ptr);
  }
};
__publicField2(_OnigString, "LAST_ID", 0);
__publicField2(_OnigString, "_sharedPtr", 0);
__publicField2(_OnigString, "_sharedPtrInUse", false);
let OnigString = _OnigString;
class OnigScanner {
  constructor(patterns) {
    __publicField2(this, "_onigBinding");
    __publicField2(this, "_ptr");
    if (!onigBinding)
      throw new ShikiError3("Must invoke loadWasm first.");
    const strPtrsArr = [];
    const strLenArr = [];
    for (let i2 = 0, len = patterns.length; i2 < len; i2++) {
      const utfString = new UtfString(patterns[i2]);
      strPtrsArr[i2] = utfString.createString(onigBinding);
      strLenArr[i2] = utfString.utf8Length;
    }
    const strPtrsPtr = onigBinding.omalloc(4 * patterns.length);
    onigBinding.HEAPU32.set(strPtrsArr, strPtrsPtr / 4);
    const strLenPtr = onigBinding.omalloc(4 * patterns.length);
    onigBinding.HEAPU32.set(strLenArr, strLenPtr / 4);
    const scannerPtr = onigBinding.createOnigScanner(strPtrsPtr, strLenPtr, patterns.length);
    for (let i2 = 0, len = patterns.length; i2 < len; i2++)
      onigBinding.ofree(strPtrsArr[i2]);
    onigBinding.ofree(strLenPtr);
    onigBinding.ofree(strPtrsPtr);
    if (scannerPtr === 0)
      throwLastOnigError(onigBinding);
    this._onigBinding = onigBinding;
    this._ptr = scannerPtr;
  }
  dispose() {
    this._onigBinding.freeOnigScanner(this._ptr);
  }
  findNextMatchSync(string, startPosition, arg) {
    let options = 0;
    if (typeof arg === "number") {
      options = arg;
    }
    if (typeof string === "string") {
      string = new OnigString(string);
      const result = this._findNextMatchSync(string, startPosition, false, options);
      string.dispose();
      return result;
    }
    return this._findNextMatchSync(string, startPosition, false, options);
  }
  _findNextMatchSync(string, startPosition, debugCall, options) {
    const onigBinding2 = this._onigBinding;
    const resultPtr = onigBinding2.findNextOnigScannerMatch(this._ptr, string.id, string.ptr, string.utf8Length, string.convertUtf16OffsetToUtf8(startPosition), options);
    if (resultPtr === 0) {
      return null;
    }
    const HEAPU32 = onigBinding2.HEAPU32;
    let offset = resultPtr / 4;
    const index = HEAPU32[offset++];
    const count = HEAPU32[offset++];
    const captureIndices = [];
    for (let i2 = 0; i2 < count; i2++) {
      const beg = string.convertUtf8OffsetToUtf16(HEAPU32[offset++]);
      const end = string.convertUtf8OffsetToUtf16(HEAPU32[offset++]);
      captureIndices[i2] = {
        start: beg,
        end,
        length: end - beg
      };
    }
    return {
      index,
      captureIndices
    };
  }
}
function isInstantiatorOptionsObject(dataOrOptions) {
  return typeof dataOrOptions.instantiator === "function";
}
function isInstantiatorModule(dataOrOptions) {
  return typeof dataOrOptions.default === "function";
}
function isDataOptionsObject(dataOrOptions) {
  return typeof dataOrOptions.data !== "undefined";
}
function isResponse(dataOrOptions) {
  return typeof Response !== "undefined" && dataOrOptions instanceof Response;
}
function isArrayBuffer(data) {
  var _a2;
  return typeof ArrayBuffer !== "undefined" && (data instanceof ArrayBuffer || ArrayBuffer.isView(data)) || typeof Buffer !== "undefined" && ((_a2 = Buffer.isBuffer) == null ? void 0 : _a2.call(Buffer, data)) || typeof SharedArrayBuffer !== "undefined" && data instanceof SharedArrayBuffer || typeof Uint32Array !== "undefined" && data instanceof Uint32Array;
}
let initPromise;
function loadWasm(options) {
  if (initPromise)
    return initPromise;
  async function _load() {
    onigBinding = await main(async (info) => {
      let instance = options;
      instance = await instance;
      if (typeof instance === "function")
        instance = await instance(info);
      if (typeof instance === "function")
        instance = await instance(info);
      if (isInstantiatorOptionsObject(instance)) {
        instance = await instance.instantiator(info);
      } else if (isInstantiatorModule(instance)) {
        instance = await instance.default(info);
      } else {
        if (isDataOptionsObject(instance))
          instance = instance.data;
        if (isResponse(instance)) {
          if (typeof WebAssembly.instantiateStreaming === "function")
            instance = await _makeResponseStreamingLoader(instance)(info);
          else
            instance = await _makeResponseNonStreamingLoader(instance)(info);
        } else if (isArrayBuffer(instance)) {
          instance = await _makeArrayBufferLoader(instance)(info);
        } else if (instance instanceof WebAssembly.Module) {
          instance = await _makeArrayBufferLoader(instance)(info);
        } else if ("default" in instance && instance.default instanceof WebAssembly.Module) {
          instance = await _makeArrayBufferLoader(instance.default)(info);
        }
      }
      if ("instance" in instance)
        instance = instance.instance;
      if ("exports" in instance)
        instance = instance.exports;
      return instance;
    });
  }
  initPromise = _load();
  return initPromise;
}
function _makeArrayBufferLoader(data) {
  return (importObject) => WebAssembly.instantiate(data, importObject);
}
function _makeResponseStreamingLoader(data) {
  return (importObject) => WebAssembly.instantiateStreaming(data, importObject);
}
function _makeResponseNonStreamingLoader(data) {
  return async (importObject) => {
    const arrayBuffer = await data.arrayBuffer();
    return WebAssembly.instantiate(arrayBuffer, importObject);
  };
}
async function createOnigurumaEngine(options) {
  if (options)
    await loadWasm(options);
  return {
    createScanner(patterns) {
      return new OnigScanner(patterns.map((p2) => typeof p2 === "string" ? p2 : p2.source));
    },
    createString(s2) {
      return new OnigString(s2);
    }
  };
}
const createHighlighter = /* @__PURE__ */ createBundledHighlighter({
  langs: bundledLanguages,
  themes: bundledThemes,
  engine: () => createOnigurumaEngine(__vitePreload(() => import("./wasm2.js"), true ? [] : void 0))
});
const {
  codeToHtml,
  codeToHast,
  codeToTokens,
  codeToTokensBase,
  codeToTokensWithThemes,
  getSingletonHighlighter,
  getLastGrammarState
} = /* @__PURE__ */ createSingletonShorthands(
  createHighlighter,
  { guessEmbeddedLanguages }
);
function r$2(e) {
  if ([...e].length !== 1) throw new Error(`Expected "${e}" to be a single code point`);
  return e.codePointAt(0);
}
function l$1(e, t, n) {
  return e.has(t) || e.set(t, n), e.get(t);
}
const i = /* @__PURE__ */ new Set(["alnum", "alpha", "ascii", "blank", "cntrl", "digit", "graph", "lower", "print", "punct", "space", "upper", "word", "xdigit"]), o$1 = String.raw;
function u(e, t) {
  if (e == null) throw new Error(t ?? "Value expected");
  return e;
}
const m$1 = o$1`\[\^?`, b$1 = `c.? | C(?:-.?)?|${o$1`[pP]\{(?:\^?[-\x20_]*[A-Za-z][-\x20\w]*\})?`}|${o$1`x[89A-Fa-f]\p{AHex}(?:\\x[89A-Fa-f]\p{AHex})*`}|${o$1`u(?:\p{AHex}{4})? | x\{[^\}]*\}? | x\p{AHex}{0,2}`}|${o$1`o\{[^\}]*\}?`}|${o$1`\d{1,3}`}`, y$1 = /[?*+][?+]?|\{(?:\d+(?:,\d*)?|,\d+)\}\??/, C$1 = new RegExp(o$1`
  \\ (?:
    ${b$1}
    | [gk]<[^>]*>?
    | [gk]'[^']*'?
    | .
  )
  | \( (?:
    \? (?:
      [:=!>({]
      | <[=!]
      | <[^>]*>
      | '[^']*'
      | ~\|?
      | #(?:[^)\\]|\\.?)*
      | [^:)]*[:)]
    )?
    | \*[^\)]*\)?
  )?
  | (?:${y$1.source})+
  | ${m$1}
  | .
`.replace(/\s+/g, ""), "gsu"), T$1 = new RegExp(o$1`
  \\ (?:
    ${b$1}
    | .
  )
  | \[:(?:\^?\p{Alpha}+|\^):\]
  | ${m$1}
  | &&
  | .
`.replace(/\s+/g, ""), "gsu");
function M$1(e, n = {}) {
  const t = { flags: "", ...n, rules: { captureGroup: false, singleline: false, ...n.rules } };
  if (typeof e != "string") throw new Error("String expected as pattern");
  const o2 = Y(t.flags), s2 = [o2.extended], a = { captureGroup: t.rules.captureGroup, getCurrentModX() {
    return s2.at(-1);
  }, numOpenGroups: 0, popModX() {
    s2.pop();
  }, pushModX(u2) {
    s2.push(u2);
  }, replaceCurrentModX(u2) {
    s2[s2.length - 1] = u2;
  }, singleline: t.rules.singleline };
  let r2 = [], i2;
  for (C$1.lastIndex = 0; i2 = C$1.exec(e); ) {
    const u2 = F$1(a, e, i2[0], C$1.lastIndex);
    u2.tokens ? r2.push(...u2.tokens) : u2.token && r2.push(u2.token), u2.lastIndex !== void 0 && (C$1.lastIndex = u2.lastIndex);
  }
  const l2 = [];
  let c = 0;
  r2.filter((u2) => u2.type === "GroupOpen").forEach((u2) => {
    u2.kind === "capturing" ? u2.number = ++c : u2.raw === "(" && l2.push(u2);
  }), c || l2.forEach((u2, S2) => {
    u2.kind = "capturing", u2.number = S2 + 1;
  });
  const g = c || l2.length;
  return { tokens: r2.map((u2) => u2.type === "EscapedNumber" ? ee$1(u2, g) : u2).flat(), flags: o2 };
}
function F$1(e, n, t, o2) {
  const [s2, a] = t;
  if (t === "[" || t === "[^") {
    const r2 = K$1(n, t, o2);
    return { tokens: r2.tokens, lastIndex: r2.lastIndex };
  }
  if (s2 === "\\") {
    if ("AbBGyYzZ".includes(a)) return { token: w$1(t, t) };
    if (/^\\g[<']/.test(t)) {
      if (!/^\\g(?:<[^>]+>|'[^']+')$/.test(t)) throw new Error(`Invalid group name "${t}"`);
      return { token: R$1(t) };
    }
    if (/^\\k[<']/.test(t)) {
      if (!/^\\k(?:<[^>]+>|'[^']+')$/.test(t)) throw new Error(`Invalid group name "${t}"`);
      return { token: A$1(t) };
    }
    if (a === "K") return { token: I$1("keep", t) };
    if (a === "N" || a === "R") return { token: k$1("newline", t, { negate: a === "N" }) };
    if (a === "O") return { token: k$1("any", t) };
    if (a === "X") return { token: k$1("text_segment", t) };
    const r2 = x(t, { inCharClass: false });
    return Array.isArray(r2) ? { tokens: r2 } : { token: r2 };
  }
  if (s2 === "(") {
    if (a === "*") return { token: j(t) };
    if (t === "(?{") throw new Error(`Unsupported callout "${t}"`);
    if (t.startsWith("(?#")) {
      if (n[o2] !== ")") throw new Error('Unclosed comment group "(?#"');
      return { lastIndex: o2 + 1 };
    }
    if (/^\(\?[-imx]+[:)]$/.test(t)) return { token: L$1(t, e) };
    if (e.pushModX(e.getCurrentModX()), e.numOpenGroups++, t === "(" && !e.captureGroup || t === "(?:") return { token: f$1("group", t) };
    if (t === "(?>") return { token: f$1("atomic", t) };
    if (t === "(?=" || t === "(?!" || t === "(?<=" || t === "(?<!") return { token: f$1(t[2] === "<" ? "lookbehind" : "lookahead", t, { negate: t.endsWith("!") }) };
    if (t === "(" && e.captureGroup || t.startsWith("(?<") && t.endsWith(">") || t.startsWith("(?'") && t.endsWith("'")) return { token: f$1("capturing", t, { ...t !== "(" && { name: t.slice(3, -1) } }) };
    if (t.startsWith("(?~")) {
      if (t === "(?~|") throw new Error(`Unsupported absence function kind "${t}"`);
      return { token: f$1("absence_repeater", t) };
    }
    throw t === "(?(" ? new Error(`Unsupported conditional "${t}"`) : new Error(`Invalid or unsupported group option "${t}"`);
  }
  if (t === ")") {
    if (e.popModX(), e.numOpenGroups--, e.numOpenGroups < 0) throw new Error('Unmatched ")"');
    return { token: Q$1(t) };
  }
  if (e.getCurrentModX()) {
    if (t === "#") {
      const r2 = n.indexOf(`
`, o2);
      return { lastIndex: r2 === -1 ? n.length : r2 };
    }
    if (/^\s$/.test(t)) {
      const r2 = /\s+/y;
      return r2.lastIndex = o2, { lastIndex: r2.exec(n) ? r2.lastIndex : o2 };
    }
  }
  if (t === ".") return { token: k$1("dot", t) };
  if (t === "^" || t === "$") {
    const r2 = e.singleline ? { "^": o$1`\A`, $: o$1`\Z` }[t] : t;
    return { token: w$1(r2, t) };
  }
  return t === "|" ? { token: P$1(t) } : y$1.test(t) ? { tokens: te$1(t) } : { token: d(r$2(t), t) };
}
function K$1(e, n, t) {
  const o2 = [E$1(n[1] === "^", n)];
  let s2 = 1, a;
  for (T$1.lastIndex = t; a = T$1.exec(e); ) {
    const r2 = a[0];
    if (r2[0] === "[" && r2[1] !== ":") s2++, o2.push(E$1(r2[1] === "^", r2));
    else if (r2 === "]") {
      if (o2.at(-1).type === "CharacterClassOpen") o2.push(d(93, r2));
      else if (s2--, o2.push(z$1(r2)), !s2) break;
    } else {
      const i2 = X$1(r2);
      Array.isArray(i2) ? o2.push(...i2) : o2.push(i2);
    }
  }
  return { tokens: o2, lastIndex: T$1.lastIndex || e.length };
}
function X$1(e) {
  if (e[0] === "\\") return x(e, { inCharClass: true });
  if (e[0] === "[") {
    const n = /\[:(?<negate>\^?)(?<name>[a-z]+):\]/.exec(e);
    if (!n || !i.has(n.groups.name)) throw new Error(`Invalid POSIX class "${e}"`);
    return k$1("posix", e, { value: n.groups.name, negate: !!n.groups.negate });
  }
  return e === "-" ? U$1(e) : e === "&&" ? H(e) : d(r$2(e), e);
}
function x(e, { inCharClass: n }) {
  const t = e[1];
  if (t === "c" || t === "C") return Z(e);
  if ("dDhHsSwW".includes(t)) return q(e);
  if (e.startsWith(o$1`\o{`)) throw new Error(`Incomplete, invalid, or unsupported octal code point "${e}"`);
  if (/^\\[pP]\{/.test(e)) {
    if (e.length === 3) throw new Error(`Incomplete or invalid Unicode property "${e}"`);
    return V$1(e);
  }
  if (new RegExp("^\\\\x[89A-Fa-f]\\p{AHex}", "u").test(e)) try {
    const o2 = e.split(/\\x/).slice(1).map((i2) => parseInt(i2, 16)), s2 = new TextDecoder("utf-8", { ignoreBOM: true, fatal: true }).decode(new Uint8Array(o2)), a = new TextEncoder();
    return [...s2].map((i2) => {
      const l2 = [...a.encode(i2)].map((c) => `\\x${c.toString(16)}`).join("");
      return d(r$2(i2), l2);
    });
  } catch {
    throw new Error(`Multibyte code "${e}" incomplete or invalid in Oniguruma`);
  }
  if (t === "u" || t === "x") return d(J$1(e), e);
  if ($$1.has(t)) return d($$1.get(t), e);
  if (/\d/.test(t)) return W$1(n, e);
  if (e === "\\") throw new Error(o$1`Incomplete escape "\"`);
  if (t === "M") throw new Error(`Unsupported meta "${e}"`);
  if ([...e].length === 2) return d(e.codePointAt(1), e);
  throw new Error(`Unexpected escape "${e}"`);
}
function P$1(e) {
  return { type: "Alternator", raw: e };
}
function w$1(e, n) {
  return { type: "Assertion", kind: e, raw: n };
}
function A$1(e) {
  return { type: "Backreference", raw: e };
}
function d(e, n) {
  return { type: "Character", value: e, raw: n };
}
function z$1(e) {
  return { type: "CharacterClassClose", raw: e };
}
function U$1(e) {
  return { type: "CharacterClassHyphen", raw: e };
}
function H(e) {
  return { type: "CharacterClassIntersector", raw: e };
}
function E$1(e, n) {
  return { type: "CharacterClassOpen", negate: e, raw: n };
}
function k$1(e, n, t = {}) {
  return { type: "CharacterSet", kind: e, ...t, raw: n };
}
function I$1(e, n, t = {}) {
  return e === "keep" ? { type: "Directive", kind: e, raw: n } : { type: "Directive", kind: e, flags: u(t.flags), raw: n };
}
function W$1(e, n) {
  return { type: "EscapedNumber", inCharClass: e, raw: n };
}
function Q$1(e) {
  return { type: "GroupClose", raw: e };
}
function f$1(e, n, t = {}) {
  return { type: "GroupOpen", kind: e, ...t, raw: n };
}
function D$1(e, n, t, o2) {
  return { type: "NamedCallout", kind: e, tag: n, arguments: t, raw: o2 };
}
function _$1(e, n, t, o2) {
  return { type: "Quantifier", kind: e, min: n, max: t, raw: o2 };
}
function R$1(e) {
  return { type: "Subroutine", raw: e };
}
const B$1 = /* @__PURE__ */ new Set(["COUNT", "CMP", "ERROR", "FAIL", "MAX", "MISMATCH", "SKIP", "TOTAL_COUNT"]), $$1 = /* @__PURE__ */ new Map([["a", 7], ["b", 8], ["e", 27], ["f", 12], ["n", 10], ["r", 13], ["t", 9], ["v", 11]]);
function Z(e) {
  const n = e[1] === "c" ? e[2] : e[3];
  if (!n || !/[A-Za-z]/.test(n)) throw new Error(`Unsupported control character "${e}"`);
  return d(r$2(n.toUpperCase()) - 64, e);
}
function L$1(e, n) {
  let { on: t, off: o2 } = /^\(\?(?<on>[imx]*)(?:-(?<off>[-imx]*))?/.exec(e).groups;
  o2 ?? (o2 = "");
  const s2 = (n.getCurrentModX() || t.includes("x")) && !o2.includes("x"), a = v(t), r2 = v(o2), i2 = {};
  if (a && (i2.enable = a), r2 && (i2.disable = r2), e.endsWith(")")) return n.replaceCurrentModX(s2), I$1("flags", e, { flags: i2 });
  if (e.endsWith(":")) return n.pushModX(s2), n.numOpenGroups++, f$1("group", e, { ...(a || r2) && { flags: i2 } });
  throw new Error(`Unexpected flag modifier "${e}"`);
}
function j(e) {
  const n = /\(\*(?<name>[A-Za-z_]\w*)?(?:\[(?<tag>(?:[A-Za-z_]\w*)?)\])?(?:\{(?<args>[^}]*)\})?\)/.exec(e);
  if (!n) throw new Error(`Incomplete or invalid named callout "${e}"`);
  const { name: t, tag: o2, args: s2 } = n.groups;
  if (!t) throw new Error(`Invalid named callout "${e}"`);
  if (o2 === "") throw new Error(`Named callout tag with empty value not allowed "${e}"`);
  const a = s2 ? s2.split(",").filter((g) => g !== "").map((g) => /^[+-]?\d+$/.test(g) ? +g : g) : [], [r2, i2, l2] = a, c = B$1.has(t) ? t.toLowerCase() : "custom";
  switch (c) {
    case "fail":
    case "mismatch":
    case "skip":
      if (a.length > 0) throw new Error(`Named callout arguments not allowed "${a}"`);
      break;
    case "error":
      if (a.length > 1) throw new Error(`Named callout allows only one argument "${a}"`);
      if (typeof r2 == "string") throw new Error(`Named callout argument must be a number "${r2}"`);
      break;
    case "max":
      if (!a.length || a.length > 2) throw new Error(`Named callout must have one or two arguments "${a}"`);
      if (typeof r2 == "string" && !/^[A-Za-z_]\w*$/.test(r2)) throw new Error(`Named callout argument one must be a tag or number "${r2}"`);
      if (a.length === 2 && (typeof i2 == "number" || !/^[<>X]$/.test(i2))) throw new Error(`Named callout optional argument two must be '<', '>', or 'X' "${i2}"`);
      break;
    case "count":
    case "total_count":
      if (a.length > 1) throw new Error(`Named callout allows only one argument "${a}"`);
      if (a.length === 1 && (typeof r2 == "number" || !/^[<>X]$/.test(r2))) throw new Error(`Named callout optional argument must be '<', '>', or 'X' "${r2}"`);
      break;
    case "cmp":
      if (a.length !== 3) throw new Error(`Named callout must have three arguments "${a}"`);
      if (typeof r2 == "string" && !/^[A-Za-z_]\w*$/.test(r2)) throw new Error(`Named callout argument one must be a tag or number "${r2}"`);
      if (typeof i2 == "number" || !/^(?:[<>!=]=|[<>])$/.test(i2)) throw new Error(`Named callout argument two must be '==', '!=', '>', '<', '>=', or '<=' "${i2}"`);
      if (typeof l2 == "string" && !/^[A-Za-z_]\w*$/.test(l2)) throw new Error(`Named callout argument three must be a tag or number "${l2}"`);
      break;
    case "custom":
      throw new Error(`Undefined callout name "${t}"`);
    default:
      throw new Error(`Unexpected named callout kind "${c}"`);
  }
  return D$1(c, o2 ?? null, (s2 == null ? void 0 : s2.split(",")) ?? null, e);
}
function O$1(e) {
  let n = null, t, o2;
  if (e[0] === "{") {
    const { minStr: s2, maxStr: a } = /^\{(?<minStr>\d*)(?:,(?<maxStr>\d*))?/.exec(e).groups, r2 = 1e5;
    if (+s2 > r2 || a && +a > r2) throw new Error("Quantifier value unsupported in Oniguruma");
    if (t = +s2, o2 = a === void 0 ? +s2 : a === "" ? 1 / 0 : +a, t > o2 && (n = "possessive", [t, o2] = [o2, t]), e.endsWith("?")) {
      if (n === "possessive") throw new Error('Unsupported possessive interval quantifier chain with "?"');
      n = "lazy";
    } else n || (n = "greedy");
  } else t = e[0] === "+" ? 1 : 0, o2 = e[0] === "?" ? 1 : 1 / 0, n = e[1] === "+" ? "possessive" : e[1] === "?" ? "lazy" : "greedy";
  return _$1(n, t, o2, e);
}
function q(e) {
  const n = e[1].toLowerCase();
  return k$1({ d: "digit", h: "hex", s: "space", w: "word" }[n], e, { negate: e[1] !== n });
}
function V$1(e) {
  const { p: n, neg: t, value: o2 } = /^\\(?<p>[pP])\{(?<neg>\^?)(?<value>[^}]+)/.exec(e).groups;
  return k$1("property", e, { value: o2, negate: n === "P" && !t || n === "p" && !!t });
}
function v(e) {
  const n = {};
  return e.includes("i") && (n.ignoreCase = true), e.includes("m") && (n.dotAll = true), e.includes("x") && (n.extended = true), Object.keys(n).length ? n : null;
}
function Y(e) {
  const n = { ignoreCase: false, dotAll: false, extended: false, digitIsAscii: false, posixIsAscii: false, spaceIsAscii: false, wordIsAscii: false, textSegmentMode: null };
  for (let t = 0; t < e.length; t++) {
    const o2 = e[t];
    if (!"imxDPSWy".includes(o2)) throw new Error(`Invalid flag "${o2}"`);
    if (o2 === "y") {
      if (!/^y{[gw]}/.test(e.slice(t))) throw new Error('Invalid or unspecified flag "y" mode');
      n.textSegmentMode = e[t + 2] === "g" ? "grapheme" : "word", t += 3;
      continue;
    }
    n[{ i: "ignoreCase", m: "dotAll", x: "extended", D: "digitIsAscii", P: "posixIsAscii", S: "spaceIsAscii", W: "wordIsAscii" }[o2]] = true;
  }
  return n;
}
function J$1(e) {
  if (new RegExp("^(?:\\\\u(?!\\p{AHex}{4})|\\\\x(?!\\p{AHex}{1,2}|\\{\\p{AHex}{1,8}\\}))", "u").test(e)) throw new Error(`Incomplete or invalid escape "${e}"`);
  const n = e[2] === "{" ? new RegExp("^\\\\x\\{\\s*(?<hex>\\p{AHex}+)", "u").exec(e).groups.hex : e.slice(2);
  return parseInt(n, 16);
}
function ee$1(e, n) {
  const { raw: t, inCharClass: o2 } = e, s2 = t.slice(1);
  if (!o2 && (s2 !== "0" && s2.length === 1 || s2[0] !== "0" && +s2 <= n)) return [A$1(t)];
  const a = [], r2 = s2.match(/^[0-7]+|\d/g);
  for (let i2 = 0; i2 < r2.length; i2++) {
    const l2 = r2[i2];
    let c;
    if (i2 === 0 && l2 !== "8" && l2 !== "9") {
      if (c = parseInt(l2, 8), c > 127) throw new Error(o$1`Octal encoded byte above 177 unsupported "${t}"`);
    } else c = r$2(l2);
    a.push(d(c, (i2 === 0 ? "\\" : "") + l2));
  }
  return a;
}
function te$1(e) {
  const n = [], t = new RegExp(y$1, "gy");
  let o2;
  for (; o2 = t.exec(e); ) {
    const s2 = o2[0];
    if (s2[0] === "{") {
      const a = /^\{(?<min>\d+),(?<max>\d+)\}\??$/.exec(s2);
      if (a) {
        const { min: r2, max: i2 } = a.groups;
        if (+r2 > +i2 && s2.endsWith("?")) {
          t.lastIndex--, n.push(O$1(s2.slice(0, -1)));
          continue;
        }
      }
    }
    n.push(O$1(s2));
  }
  return n;
}
function o(e, t) {
  if (!Array.isArray(e.body)) throw new Error("Expected node with body array");
  if (e.body.length !== 1) return false;
  const r2 = e.body[0];
  return !t || Object.keys(t).every((n) => t[n] === r2[n]);
}
function s(e) {
  return y.has(e.type);
}
const y = /* @__PURE__ */ new Set(["AbsenceFunction", "Backreference", "CapturingGroup", "Character", "CharacterClass", "CharacterSet", "Group", "Quantifier", "Subroutine"]);
function J(e, r2 = {}) {
  const n = { flags: "", normalizeUnknownPropertyNames: false, skipBackrefValidation: false, skipLookbehindValidation: false, skipPropertyNameValidation: false, unicodePropertyMap: null, ...r2, rules: { captureGroup: false, singleline: false, ...r2.rules } }, t = M$1(e, { flags: n.flags, rules: { captureGroup: n.rules.captureGroup, singleline: n.rules.singleline } }), s2 = (p2, N) => {
    const u2 = t.tokens[o2.nextIndex];
    switch (o2.parent = p2, o2.nextIndex++, u2.type) {
      case "Alternator":
        return b();
      case "Assertion":
        return W(u2);
      case "Backreference":
        return X(u2, o2);
      case "Character":
        return m(u2.value, { useLastValid: !!N.isCheckingRangeEnd });
      case "CharacterClassHyphen":
        return ee(u2, o2, N);
      case "CharacterClassOpen":
        return re(u2, o2, N);
      case "CharacterSet":
        return ne(u2, o2);
      case "Directive":
        return I(u2.kind, { flags: u2.flags });
      case "GroupOpen":
        return te(u2, o2, N);
      case "NamedCallout":
        return U(u2.kind, u2.tag, u2.arguments);
      case "Quantifier":
        return oe(u2, o2);
      case "Subroutine":
        return ae(u2, o2);
      default:
        throw new Error(`Unexpected token type "${u2.type}"`);
    }
  }, o2 = { capturingGroups: [], hasNumberedRef: false, namedGroupsByName: /* @__PURE__ */ new Map(), nextIndex: 0, normalizeUnknownPropertyNames: n.normalizeUnknownPropertyNames, parent: null, skipBackrefValidation: n.skipBackrefValidation, skipLookbehindValidation: n.skipLookbehindValidation, skipPropertyNameValidation: n.skipPropertyNameValidation, subroutines: [], tokens: t.tokens, unicodePropertyMap: n.unicodePropertyMap, walk: s2 }, i2 = B(T(t.flags));
  let d2 = i2.body[0];
  for (; o2.nextIndex < t.tokens.length; ) {
    const p2 = s2(d2, {});
    p2.type === "Alternative" ? (i2.body.push(p2), d2 = p2) : d2.body.push(p2);
  }
  const { capturingGroups: a, hasNumberedRef: l2, namedGroupsByName: c, subroutines: f2 } = o2;
  if (l2 && c.size && !n.rules.captureGroup) throw new Error("Numbered backref/subroutine not allowed when using named capture");
  for (const { ref: p2 } of f2) if (typeof p2 == "number") {
    if (p2 > a.length) throw new Error("Subroutine uses a group number that's not defined");
    p2 && (a[p2 - 1].isSubroutined = true);
  } else if (c.has(p2)) {
    if (c.get(p2).length > 1) throw new Error(o$1`Subroutine uses a duplicate group name "\g<${p2}>"`);
    c.get(p2)[0].isSubroutined = true;
  } else throw new Error(o$1`Subroutine uses a group name that's not defined "\g<${p2}>"`);
  return i2;
}
function W({ kind: e }) {
  return F(u({ "^": "line_start", $: "line_end", "\\A": "string_start", "\\b": "word_boundary", "\\B": "word_boundary", "\\G": "search_start", "\\y": "text_segment_boundary", "\\Y": "text_segment_boundary", "\\z": "string_end", "\\Z": "string_end_newline" }[e], `Unexpected assertion kind "${e}"`), { negate: e === o$1`\B` || e === o$1`\Y` });
}
function X({ raw: e }, r2) {
  const n = /^\\k[<']/.test(e), t = n ? e.slice(3, -1) : e.slice(1), s2 = (o2, i2 = false) => {
    const d2 = r2.capturingGroups.length;
    let a = false;
    if (o2 > d2) if (r2.skipBackrefValidation) a = true;
    else throw new Error(`Not enough capturing groups defined to the left "${e}"`);
    return r2.hasNumberedRef = true, k(i2 ? d2 + 1 - o2 : o2, { orphan: a });
  };
  if (n) {
    const o2 = /^(?<sign>-?)0*(?<num>[1-9]\d*)$/.exec(t);
    if (o2) return s2(+o2.groups.num, !!o2.groups.sign);
    if (/[-+]/.test(t)) throw new Error(`Invalid backref name "${e}"`);
    if (!r2.namedGroupsByName.has(t)) throw new Error(`Group name not defined to the left "${e}"`);
    return k(t);
  }
  return s2(+t);
}
function ee(e, r2, n) {
  const { tokens: t, walk: s2 } = r2, o2 = r2.parent, i2 = o2.body.at(-1), d2 = t[r2.nextIndex];
  if (!n.isCheckingRangeEnd && i2 && i2.type !== "CharacterClass" && i2.type !== "CharacterClassRange" && d2 && d2.type !== "CharacterClassOpen" && d2.type !== "CharacterClassClose" && d2.type !== "CharacterClassIntersector") {
    const a = s2(o2, { ...n, isCheckingRangeEnd: true });
    if (i2.type === "Character" && a.type === "Character") return o2.body.pop(), L(i2, a);
    throw new Error("Invalid character class range");
  }
  return m(r$2("-"));
}
function re({ negate: e }, r2, n) {
  const { tokens: t, walk: s2 } = r2, o2 = t[r2.nextIndex], i2 = [C()];
  let d2 = z(o2);
  for (; d2.type !== "CharacterClassClose"; ) {
    if (d2.type === "CharacterClassIntersector") i2.push(C()), r2.nextIndex++;
    else {
      const l2 = i2.at(-1);
      l2.body.push(s2(l2, n));
    }
    d2 = z(t[r2.nextIndex], o2);
  }
  const a = C({ negate: e });
  return i2.length === 1 ? a.body = i2[0].body : (a.kind = "intersection", a.body = i2.map((l2) => l2.body.length === 1 ? l2.body[0] : l2)), r2.nextIndex++, a;
}
function ne({ kind: e, negate: r2, value: n }, t) {
  const { normalizeUnknownPropertyNames: s2, skipPropertyNameValidation: o2, unicodePropertyMap: i$1 } = t;
  if (e === "property") {
    const d2 = w(n);
    if (i.has(d2) && !(i$1 == null ? void 0 : i$1.has(d2))) e = "posix", n = d2;
    else return Q(n, { negate: r2, normalizeUnknownPropertyNames: s2, skipPropertyNameValidation: o2, unicodePropertyMap: i$1 });
  }
  return e === "posix" ? R(n, { negate: r2 }) : E(e, { negate: r2 });
}
function te(e, r2, n) {
  const { tokens: t, capturingGroups: s2, namedGroupsByName: o2, skipLookbehindValidation: i2, walk: d2 } = r2, a = ie(e), l2 = a.type === "AbsenceFunction", c = $(a), f2 = c && a.negate;
  if (a.type === "CapturingGroup" && (s2.push(a), a.name && l$1(o2, a.name, []).push(a)), l2 && n.isInAbsenceFunction) throw new Error("Nested absence function not supported by Oniguruma");
  let p2 = D(t[r2.nextIndex]);
  for (; p2.type !== "GroupClose"; ) {
    if (p2.type === "Alternator") a.body.push(b()), r2.nextIndex++;
    else {
      const N = a.body.at(-1), u2 = d2(N, { ...n, isInAbsenceFunction: n.isInAbsenceFunction || l2, isInLookbehind: n.isInLookbehind || c, isInNegLookbehind: n.isInNegLookbehind || f2 });
      if (N.body.push(u2), (c || n.isInLookbehind) && !i2) {
        const v2 = "Lookbehind includes a pattern not allowed by Oniguruma";
        if (f2 || n.isInNegLookbehind) {
          if (M(u2) || u2.type === "CapturingGroup") throw new Error(v2);
        } else if (M(u2) || $(u2) && u2.negate) throw new Error(v2);
      }
    }
    p2 = D(t[r2.nextIndex]);
  }
  return r2.nextIndex++, a;
}
function oe({ kind: e, min: r2, max: n }, t) {
  const s$1 = t.parent, o2 = s$1.body.at(-1);
  if (!o2 || !s(o2)) throw new Error("Quantifier requires a repeatable token");
  const i2 = _(e, r2, n, o2);
  return s$1.body.pop(), i2;
}
function ae({ raw: e }, r2) {
  const { capturingGroups: n, subroutines: t } = r2;
  let s2 = e.slice(3, -1);
  const o2 = /^(?<sign>[-+]?)0*(?<num>[1-9]\d*)$/.exec(s2);
  if (o2) {
    const d2 = +o2.groups.num, a = n.length;
    if (r2.hasNumberedRef = true, s2 = { "": d2, "+": a + d2, "-": a + 1 - d2 }[o2.groups.sign], s2 < 1) throw new Error("Invalid subroutine number");
  } else s2 === "0" && (s2 = 0);
  const i2 = O(s2);
  return t.push(i2), i2;
}
function G(e, r2) {
  return { type: "AbsenceFunction", kind: e, body: h(r2 == null ? void 0 : r2.body) };
}
function b(e) {
  return { type: "Alternative", body: V(e == null ? void 0 : e.body) };
}
function F(e, r2) {
  const n = { type: "Assertion", kind: e };
  return (e === "word_boundary" || e === "text_segment_boundary") && (n.negate = !!(r2 == null ? void 0 : r2.negate)), n;
}
function k(e, r2) {
  const n = !!(r2 == null ? void 0 : r2.orphan);
  return { type: "Backreference", ref: e, ...n && { orphan: n } };
}
function P(e, r2) {
  const n = { name: void 0, isSubroutined: false, ...r2 };
  if (n.name !== void 0 && !se(n.name)) throw new Error(`Group name "${n.name}" invalid in Oniguruma`);
  return { type: "CapturingGroup", number: e, ...n.name && { name: n.name }, ...n.isSubroutined && { isSubroutined: n.isSubroutined }, body: h(r2 == null ? void 0 : r2.body) };
}
function m(e, r2) {
  const n = { useLastValid: false, ...r2 };
  if (e > 1114111) {
    const t = e.toString(16);
    if (n.useLastValid) e = 1114111;
    else throw e > 1310719 ? new Error(`Invalid code point out of range "\\x{${t}}"`) : new Error(`Invalid code point out of range in JS "\\x{${t}}"`);
  }
  return { type: "Character", value: e };
}
function C(e) {
  const r2 = { kind: "union", negate: false, ...e };
  return { type: "CharacterClass", kind: r2.kind, negate: r2.negate, body: V(e == null ? void 0 : e.body) };
}
function L(e, r2) {
  if (r2.value < e.value) throw new Error("Character class range out of order");
  return { type: "CharacterClassRange", min: e, max: r2 };
}
function E(e, r2) {
  const n = !!(r2 == null ? void 0 : r2.negate), t = { type: "CharacterSet", kind: e };
  return (e === "digit" || e === "hex" || e === "newline" || e === "space" || e === "word") && (t.negate = n), (e === "text_segment" || e === "newline" && !n) && (t.variableLength = true), t;
}
function I(e, r2 = {}) {
  if (e === "keep") return { type: "Directive", kind: e };
  if (e === "flags") return { type: "Directive", kind: e, flags: u(r2.flags) };
  throw new Error(`Unexpected directive kind "${e}"`);
}
function T(e) {
  return { type: "Flags", ...e };
}
function A(e) {
  const r2 = e == null ? void 0 : e.atomic, n = e == null ? void 0 : e.flags;
  if (r2 && n) throw new Error("Atomic group cannot have flags");
  return { type: "Group", ...r2 && { atomic: r2 }, ...n && { flags: n }, body: h(e == null ? void 0 : e.body) };
}
function K(e) {
  const r2 = { behind: false, negate: false, ...e };
  return { type: "LookaroundAssertion", kind: r2.behind ? "lookbehind" : "lookahead", negate: r2.negate, body: h(e == null ? void 0 : e.body) };
}
function U(e, r2, n) {
  return { type: "NamedCallout", kind: e, tag: r2, arguments: n };
}
function R(e, r2) {
  const n = !!(r2 == null ? void 0 : r2.negate);
  if (!i.has(e)) throw new Error(`Invalid POSIX class "${e}"`);
  return { type: "CharacterSet", kind: "posix", value: e, negate: n };
}
function _(e, r2, n, t) {
  if (r2 > n) throw new Error("Invalid reversed quantifier range");
  return { type: "Quantifier", kind: e, min: r2, max: n, body: t };
}
function B(e, r2) {
  return { type: "Regex", body: h(r2 == null ? void 0 : r2.body), flags: e };
}
function O(e) {
  return { type: "Subroutine", ref: e };
}
function Q(e, r2) {
  var _a2;
  const n = { negate: false, normalizeUnknownPropertyNames: false, skipPropertyNameValidation: false, unicodePropertyMap: null, ...r2 };
  let t = (_a2 = n.unicodePropertyMap) == null ? void 0 : _a2.get(w(e));
  if (!t) {
    if (n.normalizeUnknownPropertyNames) t = de(e);
    else if (n.unicodePropertyMap && !n.skipPropertyNameValidation) throw new Error(o$1`Invalid Unicode property "\p{${e}}"`);
  }
  return { type: "CharacterSet", kind: "property", value: t ?? e, negate: n.negate };
}
function ie({ flags: e, kind: r2, name: n, negate: t, number: s2 }) {
  switch (r2) {
    case "absence_repeater":
      return G("repeater");
    case "atomic":
      return A({ atomic: true });
    case "capturing":
      return P(s2, { name: n });
    case "group":
      return A({ flags: e });
    case "lookahead":
    case "lookbehind":
      return K({ behind: r2 === "lookbehind", negate: t });
    default:
      throw new Error(`Unexpected group kind "${r2}"`);
  }
}
function h(e) {
  if (e === void 0) e = [b()];
  else if (!Array.isArray(e) || !e.length || !e.every((r2) => r2.type === "Alternative")) throw new Error("Invalid body; expected array of one or more Alternative nodes");
  return e;
}
function V(e) {
  if (e === void 0) e = [];
  else if (!Array.isArray(e) || !e.every((r2) => !!r2.type)) throw new Error("Invalid body; expected array of nodes");
  return e;
}
function M(e) {
  return e.type === "LookaroundAssertion" && e.kind === "lookahead";
}
function $(e) {
  return e.type === "LookaroundAssertion" && e.kind === "lookbehind";
}
function se(e) {
  return /^[\p{Alpha}\p{Pc}][^)]*$/u.test(e);
}
function de(e) {
  return e.trim().replace(/[- _]+/g, "_").replace(/[A-Z][a-z]+(?=[A-Z])/g, "$&_").replace(/[A-Za-z]+/g, (r2) => r2[0].toUpperCase() + r2.slice(1).toLowerCase());
}
function w(e) {
  return e.replace(/[- _]+/g, "").toLowerCase();
}
function z(e, r2) {
  return u(e, `${(r2 == null ? void 0 : r2.type) === "Character" && r2.value === 93 ? "Empty" : "Unclosed"} character class`);
}
function D(e) {
  return u(e, "Unclosed group");
}
function S(a, v2, N = null) {
  function u$1(e, s2) {
    for (let t = 0; t < e.length; t++) {
      const r2 = n(e[t], s2, t, e);
      t = Math.max(-1, t + r2);
    }
  }
  function n(e, s2 = null, t = null, r2 = null) {
    var _a2, _b2;
    let i2 = 0, c = false;
    const d2 = { node: e, parent: s2, key: t, container: r2, root: a, remove() {
      f(r2).splice(Math.max(0, l(t) + i2), 1), i2--, c = true;
    }, removeAllNextSiblings() {
      return f(r2).splice(l(t) + 1);
    }, removeAllPrevSiblings() {
      const o2 = l(t) + i2;
      return i2 -= o2, f(r2).splice(0, Math.max(0, o2));
    }, replaceWith(o2, y2 = {}) {
      const b2 = !!y2.traverse;
      r2 ? r2[Math.max(0, l(t) + i2)] = o2 : u(s2, "Can't replace root node")[t] = o2, b2 && n(o2, s2, t, r2), c = true;
    }, replaceWithMultiple(o2, y2 = {}) {
      const b2 = !!y2.traverse;
      if (f(r2).splice(Math.max(0, l(t) + i2), 1, ...o2), i2 += o2.length - 1, b2) {
        let g = 0;
        for (let x2 = 0; x2 < o2.length; x2++) g += n(o2[x2], s2, l(t) + x2 + g, r2);
      }
      c = true;
    }, skip() {
      c = true;
    } }, { type: m2 } = e, h2 = v2["*"], p2 = v2[m2], R2 = typeof h2 == "function" ? h2 : h2 == null ? void 0 : h2.enter, P2 = typeof p2 == "function" ? p2 : p2 == null ? void 0 : p2.enter;
    if (R2 == null ? void 0 : R2(d2, N), P2 == null ? void 0 : P2(d2, N), !c) switch (m2) {
      case "AbsenceFunction":
      case "CapturingGroup":
      case "Group":
        u$1(e.body, e);
        break;
      case "Alternative":
      case "CharacterClass":
        u$1(e.body, e);
        break;
      case "Assertion":
      case "Backreference":
      case "Character":
      case "CharacterSet":
      case "Directive":
      case "Flags":
      case "NamedCallout":
      case "Subroutine":
        break;
      case "CharacterClassRange":
        n(e.min, e, "min"), n(e.max, e, "max");
        break;
      case "LookaroundAssertion":
        u$1(e.body, e);
        break;
      case "Quantifier":
        n(e.body, e, "body");
        break;
      case "Regex":
        u$1(e.body, e), n(e.flags, e, "flags");
        break;
      default:
        throw new Error(`Unexpected node type "${m2}"`);
    }
    return (_a2 = p2 == null ? void 0 : p2.exit) == null ? void 0 : _a2.call(p2, d2, N), (_b2 = h2 == null ? void 0 : h2.exit) == null ? void 0 : _b2.call(h2, d2, N), i2;
  }
  return n(a), a;
}
function f(a) {
  if (!Array.isArray(a)) throw new Error("Container expected");
  return a;
}
function l(a) {
  if (typeof a != "number") throw new Error("Numeric key expected");
  return a;
}
const noncapturingDelim = String.raw`\(\?(?:[:=!>A-Za-z\-]|<[=!]|\(DEFINE\))`;
function incrementIfAtLeast$1(arr, threshold) {
  for (let i2 = 0; i2 < arr.length; i2++) {
    if (arr[i2] >= threshold) {
      arr[i2]++;
    }
  }
}
function spliceStr(str, pos, oldValue, newValue) {
  return str.slice(0, pos) + newValue + str.slice(pos + oldValue.length);
}
const Context = Object.freeze({
  DEFAULT: "DEFAULT",
  CHAR_CLASS: "CHAR_CLASS"
});
function replaceUnescaped(expression, needle, replacement, context) {
  const re2 = new RegExp(String.raw`${needle}|(?<$skip>\[\^?|\\?.)`, "gsu");
  const negated = [false];
  let numCharClassesOpen = 0;
  let result = "";
  for (const match of expression.matchAll(re2)) {
    const { 0: m2, groups: { $skip } } = match;
    if (!$skip && (!context || context === Context.DEFAULT === !numCharClassesOpen)) {
      if (replacement instanceof Function) {
        result += replacement(match, {
          context: numCharClassesOpen ? Context.CHAR_CLASS : Context.DEFAULT,
          negated: negated[negated.length - 1]
        });
      } else {
        result += replacement;
      }
      continue;
    }
    if (m2[0] === "[") {
      numCharClassesOpen++;
      negated.push(m2[1] === "^");
    } else if (m2 === "]" && numCharClassesOpen) {
      numCharClassesOpen--;
      negated.pop();
    }
    result += m2;
  }
  return result;
}
function forEachUnescaped(expression, needle, callback, context) {
  replaceUnescaped(expression, needle, callback, context);
}
function execUnescaped(expression, needle, pos = 0, context) {
  if (!new RegExp(needle, "su").test(expression)) {
    return null;
  }
  const re2 = new RegExp(`${needle}|(?<$skip>\\\\?.)`, "gsu");
  re2.lastIndex = pos;
  let numCharClassesOpen = 0;
  let match;
  while (match = re2.exec(expression)) {
    const { 0: m2, groups: { $skip } } = match;
    if (!$skip && (!context || context === Context.DEFAULT === !numCharClassesOpen)) {
      return match;
    }
    if (m2 === "[") {
      numCharClassesOpen++;
    } else if (m2 === "]" && numCharClassesOpen) {
      numCharClassesOpen--;
    }
    if (re2.lastIndex == match.index) {
      re2.lastIndex++;
    }
  }
  return null;
}
function hasUnescaped(expression, needle, context) {
  return !!execUnescaped(expression, needle, 0, context);
}
function getGroupContents(expression, contentsStartPos) {
  const token2 = /\\?./gsu;
  token2.lastIndex = contentsStartPos;
  let contentsEndPos = expression.length;
  let numCharClassesOpen = 0;
  let numGroupsOpen = 1;
  let match;
  while (match = token2.exec(expression)) {
    const [m2] = match;
    if (m2 === "[") {
      numCharClassesOpen++;
    } else if (!numCharClassesOpen) {
      if (m2 === "(") {
        numGroupsOpen++;
      } else if (m2 === ")") {
        numGroupsOpen--;
        if (!numGroupsOpen) {
          contentsEndPos = match.index;
          break;
        }
      }
    } else if (m2 === "]") {
      numCharClassesOpen--;
    }
  }
  return expression.slice(contentsStartPos, contentsEndPos);
}
const atomicPluginToken = new RegExp(String.raw`(?<noncapturingStart>${noncapturingDelim})|(?<capturingStart>\((?:\?<[^>]+>)?)|\\?.`, "gsu");
function atomic(expression, data) {
  const hiddenCaptures = (data == null ? void 0 : data.hiddenCaptures) ?? [];
  let captureTransfers = (data == null ? void 0 : data.captureTransfers) ?? /* @__PURE__ */ new Map();
  if (!/\(\?>/.test(expression)) {
    return {
      pattern: expression,
      captureTransfers,
      hiddenCaptures
    };
  }
  const aGDelim = "(?>";
  const emulatedAGDelim = "(?:(?=(";
  const captureNumMap = [0];
  const addedHiddenCaptures = [];
  let numCapturesBeforeAG = 0;
  let numAGs = 0;
  let aGPos = NaN;
  let hasProcessedAG;
  do {
    hasProcessedAG = false;
    let numCharClassesOpen = 0;
    let numGroupsOpenInAG = 0;
    let inAG = false;
    let match;
    atomicPluginToken.lastIndex = Number.isNaN(aGPos) ? 0 : aGPos + emulatedAGDelim.length;
    while (match = atomicPluginToken.exec(expression)) {
      const { 0: m2, index, groups: { capturingStart, noncapturingStart } } = match;
      if (m2 === "[") {
        numCharClassesOpen++;
      } else if (!numCharClassesOpen) {
        if (m2 === aGDelim && !inAG) {
          aGPos = index;
          inAG = true;
        } else if (inAG && noncapturingStart) {
          numGroupsOpenInAG++;
        } else if (capturingStart) {
          if (inAG) {
            numGroupsOpenInAG++;
          } else {
            numCapturesBeforeAG++;
            captureNumMap.push(numCapturesBeforeAG + numAGs);
          }
        } else if (m2 === ")" && inAG) {
          if (!numGroupsOpenInAG) {
            numAGs++;
            const addedCaptureNum = numCapturesBeforeAG + numAGs;
            expression = `${expression.slice(0, aGPos)}${emulatedAGDelim}${expression.slice(aGPos + aGDelim.length, index)}))<$$${addedCaptureNum}>)${expression.slice(index + 1)}`;
            hasProcessedAG = true;
            addedHiddenCaptures.push(addedCaptureNum);
            incrementIfAtLeast$1(hiddenCaptures, addedCaptureNum);
            if (captureTransfers.size) {
              const newCaptureTransfers = /* @__PURE__ */ new Map();
              captureTransfers.forEach((from, to) => {
                newCaptureTransfers.set(
                  to >= addedCaptureNum ? to + 1 : to,
                  from.map((f2) => f2 >= addedCaptureNum ? f2 + 1 : f2)
                );
              });
              captureTransfers = newCaptureTransfers;
            }
            break;
          }
          numGroupsOpenInAG--;
        }
      } else if (m2 === "]") {
        numCharClassesOpen--;
      }
    }
  } while (hasProcessedAG);
  hiddenCaptures.push(...addedHiddenCaptures);
  expression = replaceUnescaped(
    expression,
    String.raw`\\(?<backrefNum>[1-9]\d*)|<\$\$(?<wrappedBackrefNum>\d+)>`,
    ({ 0: m2, groups: { backrefNum, wrappedBackrefNum } }) => {
      if (backrefNum) {
        const bNum = +backrefNum;
        if (bNum > captureNumMap.length - 1) {
          throw new Error(`Backref "${m2}" greater than number of captures`);
        }
        return `\\${captureNumMap[bNum]}`;
      }
      return `\\${wrappedBackrefNum}`;
    },
    Context.DEFAULT
  );
  return {
    pattern: expression,
    captureTransfers,
    hiddenCaptures
  };
}
const baseQuantifier = String.raw`(?:[?*+]|\{\d+(?:,\d*)?\})`;
const possessivePluginToken = new RegExp(String.raw`
\\(?: \d+
  | c[A-Za-z]
  | [gk]<[^>]+>
  | [pPu]\{[^\}]+\}
  | u[A-Fa-f\d]{4}
  | x[A-Fa-f\d]{2}
  )
| \((?: \? (?: [:=!>]
  | <(?:[=!]|[^>]+>)
  | [A-Za-z\-]+:
  | \(DEFINE\)
  ))?
| (?<qBase>${baseQuantifier})(?<qMod>[?+]?)(?<invalidQ>[?*+\{]?)
| \\?.
`.replace(/\s+/g, ""), "gsu");
function possessive(expression) {
  if (!new RegExp(`${baseQuantifier}\\+`).test(expression)) {
    return {
      pattern: expression
    };
  }
  const openGroupIndices = [];
  let lastGroupIndex = null;
  let lastCharClassIndex = null;
  let lastToken = "";
  let numCharClassesOpen = 0;
  let match;
  possessivePluginToken.lastIndex = 0;
  while (match = possessivePluginToken.exec(expression)) {
    const { 0: m2, index, groups: { qBase, qMod, invalidQ } } = match;
    if (m2 === "[") {
      if (!numCharClassesOpen) {
        lastCharClassIndex = index;
      }
      numCharClassesOpen++;
    } else if (m2 === "]") {
      if (numCharClassesOpen) {
        numCharClassesOpen--;
      } else {
        lastCharClassIndex = null;
      }
    } else if (!numCharClassesOpen) {
      if (qMod === "+" && lastToken && !lastToken.startsWith("(")) {
        if (invalidQ) {
          throw new Error(`Invalid quantifier "${m2}"`);
        }
        let charsAdded = -1;
        if (/^\{\d+\}$/.test(qBase)) {
          expression = spliceStr(expression, index + qBase.length, qMod, "");
        } else {
          if (lastToken === ")" || lastToken === "]") {
            const nodeIndex = lastToken === ")" ? lastGroupIndex : lastCharClassIndex;
            if (nodeIndex === null) {
              throw new Error(`Invalid unmatched "${lastToken}"`);
            }
            expression = `${expression.slice(0, nodeIndex)}(?>${expression.slice(nodeIndex, index)}${qBase})${expression.slice(index + m2.length)}`;
          } else {
            expression = `${expression.slice(0, index - lastToken.length)}(?>${lastToken}${qBase})${expression.slice(index + m2.length)}`;
          }
          charsAdded += 4;
        }
        possessivePluginToken.lastIndex += charsAdded;
      } else if (m2[0] === "(") {
        openGroupIndices.push(index);
      } else if (m2 === ")") {
        lastGroupIndex = openGroupIndices.length ? openGroupIndices.pop() : null;
      }
    }
    lastToken = m2;
  }
  return {
    pattern: expression
  };
}
const r$1 = String.raw;
const gRToken = r$1`\\g<(?<gRNameOrNum>[^>&]+)&R=(?<gRDepth>[^>]+)>`;
const recursiveToken = r$1`\(\?R=(?<rDepth>[^\)]+)\)|${gRToken}`;
const namedCaptureDelim = r$1`\(\?<(?![=!])(?<captureName>[^>]+)>`;
const captureDelim = r$1`${namedCaptureDelim}|(?<unnamed>\()(?!\?)`;
const token = new RegExp(r$1`${namedCaptureDelim}|${recursiveToken}|\(\?|\\?.`, "gsu");
const overlappingRecursionMsg = "Cannot use multiple overlapping recursions";
function recursion(pattern, data) {
  const { hiddenCaptures, mode } = {
    hiddenCaptures: [],
    mode: "plugin",
    ...data
  };
  let captureTransfers = (data == null ? void 0 : data.captureTransfers) ?? /* @__PURE__ */ new Map();
  if (!new RegExp(recursiveToken, "su").test(pattern)) {
    return {
      pattern,
      captureTransfers,
      hiddenCaptures
    };
  }
  if (mode === "plugin" && hasUnescaped(pattern, r$1`\(\?\(DEFINE\)`, Context.DEFAULT)) {
    throw new Error("DEFINE groups cannot be used with recursion");
  }
  const addedHiddenCaptures = [];
  const hasNumberedBackref = hasUnescaped(pattern, r$1`\\[1-9]`, Context.DEFAULT);
  const groupContentsStartPos = /* @__PURE__ */ new Map();
  const openGroups = [];
  let hasRecursed = false;
  let numCharClassesOpen = 0;
  let numCapturesPassed = 0;
  let match;
  token.lastIndex = 0;
  while (match = token.exec(pattern)) {
    const { 0: m2, groups: { captureName, rDepth, gRNameOrNum, gRDepth } } = match;
    if (m2 === "[") {
      numCharClassesOpen++;
    } else if (!numCharClassesOpen) {
      if (rDepth) {
        assertMaxInBounds(rDepth);
        if (hasRecursed) {
          throw new Error(overlappingRecursionMsg);
        }
        if (hasNumberedBackref) {
          throw new Error(
            // When used in `external` mode by transpilers other than Regex+, backrefs might have
            // gone through conversion from named to numbered, so avoid a misleading error
            `${mode === "external" ? "Backrefs" : "Numbered backrefs"} cannot be used with global recursion`
          );
        }
        const left = pattern.slice(0, match.index);
        const right = pattern.slice(token.lastIndex);
        if (hasUnescaped(right, recursiveToken, Context.DEFAULT)) {
          throw new Error(overlappingRecursionMsg);
        }
        const reps = +rDepth - 1;
        pattern = makeRecursive(
          left,
          right,
          reps,
          false,
          hiddenCaptures,
          addedHiddenCaptures,
          numCapturesPassed
        );
        captureTransfers = mapCaptureTransfers(
          captureTransfers,
          left,
          reps,
          addedHiddenCaptures.length,
          0,
          numCapturesPassed
        );
        break;
      } else if (gRNameOrNum) {
        assertMaxInBounds(gRDepth);
        let isWithinReffedGroup = false;
        for (const g of openGroups) {
          if (g.name === gRNameOrNum || g.num === +gRNameOrNum) {
            isWithinReffedGroup = true;
            if (g.hasRecursedWithin) {
              throw new Error(overlappingRecursionMsg);
            }
            break;
          }
        }
        if (!isWithinReffedGroup) {
          throw new Error(r$1`Recursive \g cannot be used outside the referenced group "${mode === "external" ? gRNameOrNum : r$1`\g<${gRNameOrNum}&R=${gRDepth}>`}"`);
        }
        const startPos = groupContentsStartPos.get(gRNameOrNum);
        const groupContents = getGroupContents(pattern, startPos);
        if (hasNumberedBackref && hasUnescaped(groupContents, r$1`${namedCaptureDelim}|\((?!\?)`, Context.DEFAULT)) {
          throw new Error(
            // When used in `external` mode by transpilers other than Regex+, backrefs might have
            // gone through conversion from named to numbered, so avoid a misleading error
            `${mode === "external" ? "Backrefs" : "Numbered backrefs"} cannot be used with recursion of capturing groups`
          );
        }
        const groupContentsLeft = pattern.slice(startPos, match.index);
        const groupContentsRight = groupContents.slice(groupContentsLeft.length + m2.length);
        const numAddedHiddenCapturesPreExpansion = addedHiddenCaptures.length;
        const reps = +gRDepth - 1;
        const expansion = makeRecursive(
          groupContentsLeft,
          groupContentsRight,
          reps,
          true,
          hiddenCaptures,
          addedHiddenCaptures,
          numCapturesPassed
        );
        captureTransfers = mapCaptureTransfers(
          captureTransfers,
          groupContentsLeft,
          reps,
          addedHiddenCaptures.length - numAddedHiddenCapturesPreExpansion,
          numAddedHiddenCapturesPreExpansion,
          numCapturesPassed
        );
        const pre = pattern.slice(0, startPos);
        const post = pattern.slice(startPos + groupContents.length);
        pattern = `${pre}${expansion}${post}`;
        token.lastIndex += expansion.length - m2.length - groupContentsLeft.length - groupContentsRight.length;
        openGroups.forEach((g) => g.hasRecursedWithin = true);
        hasRecursed = true;
      } else if (captureName) {
        numCapturesPassed++;
        groupContentsStartPos.set(String(numCapturesPassed), token.lastIndex);
        groupContentsStartPos.set(captureName, token.lastIndex);
        openGroups.push({
          num: numCapturesPassed,
          name: captureName
        });
      } else if (m2[0] === "(") {
        const isUnnamedCapture = m2 === "(";
        if (isUnnamedCapture) {
          numCapturesPassed++;
          groupContentsStartPos.set(String(numCapturesPassed), token.lastIndex);
        }
        openGroups.push(isUnnamedCapture ? { num: numCapturesPassed } : {});
      } else if (m2 === ")") {
        openGroups.pop();
      }
    } else if (m2 === "]") {
      numCharClassesOpen--;
    }
  }
  hiddenCaptures.push(...addedHiddenCaptures);
  return {
    pattern,
    captureTransfers,
    hiddenCaptures
  };
}
function assertMaxInBounds(max) {
  const errMsg = `Max depth must be integer between 2 and 100; used ${max}`;
  if (!/^[1-9]\d*$/.test(max)) {
    throw new Error(errMsg);
  }
  max = +max;
  if (max < 2 || max > 100) {
    throw new Error(errMsg);
  }
}
function makeRecursive(left, right, reps, isSubpattern, hiddenCaptures, addedHiddenCaptures, numCapturesPassed) {
  const namesInRecursed = /* @__PURE__ */ new Set();
  if (isSubpattern) {
    forEachUnescaped(left + right, namedCaptureDelim, ({ groups: { captureName } }) => {
      namesInRecursed.add(captureName);
    }, Context.DEFAULT);
  }
  const rest = [
    reps,
    isSubpattern ? namesInRecursed : null,
    hiddenCaptures,
    addedHiddenCaptures,
    numCapturesPassed
  ];
  return `${left}${repeatWithDepth(`(?:${left}`, "forward", ...rest)}(?:)${repeatWithDepth(`${right})`, "backward", ...rest)}${right}`;
}
function repeatWithDepth(pattern, direction, reps, namesInRecursed, hiddenCaptures, addedHiddenCaptures, numCapturesPassed) {
  const startNum = 2;
  const getDepthNum = (i2) => direction === "forward" ? i2 + startNum : reps - i2 + startNum - 1;
  let result = "";
  for (let i2 = 0; i2 < reps; i2++) {
    const depthNum = getDepthNum(i2);
    result += replaceUnescaped(
      pattern,
      r$1`${captureDelim}|\\k<(?<backref>[^>]+)>`,
      ({ 0: m2, groups: { captureName, unnamed, backref } }) => {
        if (backref && namesInRecursed && !namesInRecursed.has(backref)) {
          return m2;
        }
        const suffix = `_$${depthNum}`;
        if (unnamed || captureName) {
          const addedCaptureNum = numCapturesPassed + addedHiddenCaptures.length + 1;
          addedHiddenCaptures.push(addedCaptureNum);
          incrementIfAtLeast(hiddenCaptures, addedCaptureNum);
          return unnamed ? m2 : `(?<${captureName}${suffix}>`;
        }
        return r$1`\k<${backref}${suffix}>`;
      },
      Context.DEFAULT
    );
  }
  return result;
}
function incrementIfAtLeast(arr, threshold) {
  for (let i2 = 0; i2 < arr.length; i2++) {
    if (arr[i2] >= threshold) {
      arr[i2]++;
    }
  }
}
function mapCaptureTransfers(captureTransfers, left, reps, numCapturesAddedInExpansion, numAddedHiddenCapturesPreExpansion, numCapturesPassed) {
  if (captureTransfers.size && numCapturesAddedInExpansion) {
    let numCapturesInLeft = 0;
    forEachUnescaped(left, captureDelim, () => numCapturesInLeft++, Context.DEFAULT);
    const recursionDelimCaptureNum = numCapturesPassed - numCapturesInLeft + numAddedHiddenCapturesPreExpansion;
    const newCaptureTransfers = /* @__PURE__ */ new Map();
    captureTransfers.forEach((from, to) => {
      const numCapturesInRight = (numCapturesAddedInExpansion - numCapturesInLeft * reps) / reps;
      const numCapturesAddedInLeft = numCapturesInLeft * reps;
      const newTo = to > recursionDelimCaptureNum + numCapturesInLeft ? to + numCapturesAddedInExpansion : to;
      const newFrom = [];
      for (const f2 of from) {
        if (f2 <= recursionDelimCaptureNum) {
          newFrom.push(f2);
        } else if (f2 > recursionDelimCaptureNum + numCapturesInLeft + numCapturesInRight) {
          newFrom.push(f2 + numCapturesAddedInExpansion);
        } else if (f2 <= recursionDelimCaptureNum + numCapturesInLeft) {
          for (let i2 = 0; i2 <= reps; i2++) {
            newFrom.push(f2 + numCapturesInLeft * i2);
          }
        } else {
          for (let i2 = 0; i2 <= reps; i2++) {
            newFrom.push(f2 + numCapturesAddedInLeft + numCapturesInRight * i2);
          }
        }
      }
      newCaptureTransfers.set(newTo, newFrom);
    });
    return newCaptureTransfers;
  }
  return captureTransfers;
}
var cp = String.fromCodePoint;
var r = String.raw;
var envFlags = {
  flagGroups: (() => {
    try {
      new RegExp("(?i:)");
    } catch {
      return false;
    }
    return true;
  })(),
  unicodeSets: (() => {
    try {
      new RegExp("[[]]", "v");
    } catch {
      return false;
    }
    return true;
  })()
};
envFlags.bugFlagVLiteralHyphenIsRange = envFlags.unicodeSets ? (() => {
  try {
    new RegExp(r`[\d\-a]`, "v");
  } catch {
    return true;
  }
  return false;
})() : false;
envFlags.bugNestedClassIgnoresNegation = envFlags.unicodeSets && new RegExp("[[^a]]", "v").test("a");
function getNewCurrentFlags(current, { enable, disable }) {
  return {
    dotAll: !(disable == null ? void 0 : disable.dotAll) && !!((enable == null ? void 0 : enable.dotAll) || current.dotAll),
    ignoreCase: !(disable == null ? void 0 : disable.ignoreCase) && !!((enable == null ? void 0 : enable.ignoreCase) || current.ignoreCase)
  };
}
function getOrInsert(map, key2, defaultValue) {
  if (!map.has(key2)) {
    map.set(key2, defaultValue);
  }
  return map.get(key2);
}
function isMinTarget(target, min) {
  return EsVersion[target] >= EsVersion[min];
}
function throwIfNullish(value, msg) {
  if (value == null) {
    throw new Error(msg ?? "Value expected");
  }
  return value;
}
var EsVersion = {
  ES2025: 2025,
  ES2024: 2024,
  ES2018: 2018
};
var Target = (
  /** @type {const} */
  {
    auto: "auto",
    ES2025: "ES2025",
    ES2024: "ES2024",
    ES2018: "ES2018"
  }
);
function getOptions(options = {}) {
  if ({}.toString.call(options) !== "[object Object]") {
    throw new Error("Unexpected options");
  }
  if (options.target !== void 0 && !Target[options.target]) {
    throw new Error(`Unexpected target "${options.target}"`);
  }
  const opts = {
    // Sets the level of emulation rigor/strictness.
    accuracy: "default",
    // Disables advanced emulation that relies on returning a `RegExp` subclass, resulting in
    // certain patterns not being emulatable.
    avoidSubclass: false,
    // Oniguruma flags; a string with `i`, `m`, `x`, `D`, `S`, `W`, `y{g}` in any order (all
    // optional). Oniguruma's `m` is equivalent to JavaScript's `s` (`dotAll`).
    flags: "",
    // Include JavaScript flag `g` (`global`) in the result.
    global: false,
    // Include JavaScript flag `d` (`hasIndices`) in the result.
    hasIndices: false,
    // Delay regex construction until first use if the transpiled pattern is at least this length.
    lazyCompileLength: Infinity,
    // JavaScript version used for generated regexes. Using `auto` detects the best value based on
    // your environment. Later targets allow faster processing, simpler generated source, and
    // support for additional features.
    target: "auto",
    // Disables minifications that simplify the pattern without changing the meaning.
    verbose: false,
    ...options,
    // Advanced options that override standard behavior, error checking, and flags when enabled.
    rules: {
      // Useful with TextMate grammars that merge backreferences across patterns.
      allowOrphanBackrefs: false,
      // Use ASCII `\b` and `\B`, which increases search performance of generated regexes.
      asciiWordBoundaries: false,
      // Allow unnamed captures and numbered calls (backreferences and subroutines) when using
      // named capture. This is Oniguruma option `ONIG_OPTION_CAPTURE_GROUP`; on by default in
      // `vscode-oniguruma`.
      captureGroup: false,
      // Change the recursion depth limit from Oniguruma's `20` to an integer `2`–`20`.
      recursionLimit: 20,
      // `^` as `\A`; `$` as`\Z`. Improves search performance of generated regexes without changing
      // the meaning if searching line by line. This is Oniguruma option `ONIG_OPTION_SINGLELINE`.
      singleline: false,
      ...options.rules
    }
  };
  if (opts.target === "auto") {
    opts.target = envFlags.flagGroups ? "ES2025" : envFlags.unicodeSets ? "ES2024" : "ES2018";
  }
  return opts;
}
var asciiSpaceChar = "[	-\r ]";
var CharsWithoutIgnoreCaseExpansion = /* @__PURE__ */ new Set([
  cp(304),
  // İ
  cp(305)
  // ı
]);
var defaultWordChar = r`[\p{L}\p{M}\p{N}\p{Pc}]`;
function getIgnoreCaseMatchChars(char) {
  if (CharsWithoutIgnoreCaseExpansion.has(char)) {
    return [char];
  }
  const set = /* @__PURE__ */ new Set();
  const lower = char.toLowerCase();
  const upper = lower.toUpperCase();
  const title = LowerToTitleCaseMap.get(lower);
  const altLower = LowerToAlternativeLowerCaseMap.get(lower);
  const altUpper = LowerToAlternativeUpperCaseMap.get(lower);
  if ([...upper].length === 1) {
    set.add(upper);
  }
  altUpper && set.add(altUpper);
  title && set.add(title);
  set.add(lower);
  altLower && set.add(altLower);
  return [...set];
}
var JsUnicodePropertyMap = /* @__PURE__ */ new Map(
  `C Other
Cc Control cntrl
Cf Format
Cn Unassigned
Co Private_Use
Cs Surrogate
L Letter
LC Cased_Letter
Ll Lowercase_Letter
Lm Modifier_Letter
Lo Other_Letter
Lt Titlecase_Letter
Lu Uppercase_Letter
M Mark Combining_Mark
Mc Spacing_Mark
Me Enclosing_Mark
Mn Nonspacing_Mark
N Number
Nd Decimal_Number digit
Nl Letter_Number
No Other_Number
P Punctuation punct
Pc Connector_Punctuation
Pd Dash_Punctuation
Pe Close_Punctuation
Pf Final_Punctuation
Pi Initial_Punctuation
Po Other_Punctuation
Ps Open_Punctuation
S Symbol
Sc Currency_Symbol
Sk Modifier_Symbol
Sm Math_Symbol
So Other_Symbol
Z Separator
Zl Line_Separator
Zp Paragraph_Separator
Zs Space_Separator
ASCII
ASCII_Hex_Digit AHex
Alphabetic Alpha
Any
Assigned
Bidi_Control Bidi_C
Bidi_Mirrored Bidi_M
Case_Ignorable CI
Cased
Changes_When_Casefolded CWCF
Changes_When_Casemapped CWCM
Changes_When_Lowercased CWL
Changes_When_NFKC_Casefolded CWKCF
Changes_When_Titlecased CWT
Changes_When_Uppercased CWU
Dash
Default_Ignorable_Code_Point DI
Deprecated Dep
Diacritic Dia
Emoji
Emoji_Component EComp
Emoji_Modifier EMod
Emoji_Modifier_Base EBase
Emoji_Presentation EPres
Extended_Pictographic ExtPict
Extender Ext
Grapheme_Base Gr_Base
Grapheme_Extend Gr_Ext
Hex_Digit Hex
IDS_Binary_Operator IDSB
IDS_Trinary_Operator IDST
ID_Continue IDC
ID_Start IDS
Ideographic Ideo
Join_Control Join_C
Logical_Order_Exception LOE
Lowercase Lower
Math
Noncharacter_Code_Point NChar
Pattern_Syntax Pat_Syn
Pattern_White_Space Pat_WS
Quotation_Mark QMark
Radical
Regional_Indicator RI
Sentence_Terminal STerm
Soft_Dotted SD
Terminal_Punctuation Term
Unified_Ideograph UIdeo
Uppercase Upper
Variation_Selector VS
White_Space space
XID_Continue XIDC
XID_Start XIDS`.split(/\s/).map((p2) => [w(p2), p2])
);
var LowerToAlternativeLowerCaseMap = /* @__PURE__ */ new Map([
  ["s", cp(383)],
  // s, ſ
  [cp(383), "s"]
  // ſ, s
]);
var LowerToAlternativeUpperCaseMap = /* @__PURE__ */ new Map([
  [cp(223), cp(7838)],
  // ß, ẞ
  [cp(107), cp(8490)],
  // k, K (Kelvin)
  [cp(229), cp(8491)],
  // å, Å (Angstrom)
  [cp(969), cp(8486)]
  // ω, Ω (Ohm)
]);
var LowerToTitleCaseMap = new Map([
  titleEntry(453),
  titleEntry(456),
  titleEntry(459),
  titleEntry(498),
  ...titleRange(8072, 8079),
  ...titleRange(8088, 8095),
  ...titleRange(8104, 8111),
  titleEntry(8124),
  titleEntry(8140),
  titleEntry(8188)
]);
var PosixClassMap = /* @__PURE__ */ new Map([
  ["alnum", r`[\p{Alpha}\p{Nd}]`],
  ["alpha", r`\p{Alpha}`],
  ["ascii", r`\p{ASCII}`],
  ["blank", r`[\p{Zs}\t]`],
  ["cntrl", r`\p{Cc}`],
  ["digit", r`\p{Nd}`],
  ["graph", r`[\P{space}&&\P{Cc}&&\P{Cn}&&\P{Cs}]`],
  ["lower", r`\p{Lower}`],
  ["print", r`[[\P{space}&&\P{Cc}&&\P{Cn}&&\P{Cs}]\p{Zs}]`],
  ["punct", r`[\p{P}\p{S}]`],
  // Updated value from Onig 6.9.9; changed from Unicode `\p{punct}`
  ["space", r`\p{space}`],
  ["upper", r`\p{Upper}`],
  ["word", r`[\p{Alpha}\p{M}\p{Nd}\p{Pc}]`],
  ["xdigit", r`\p{AHex}`]
]);
function range(start, end) {
  const range2 = [];
  for (let i2 = start; i2 <= end; i2++) {
    range2.push(i2);
  }
  return range2;
}
function titleEntry(codePoint) {
  const char = cp(codePoint);
  return [char.toLowerCase(), char];
}
function titleRange(start, end) {
  return range(start, end).map((codePoint) => titleEntry(codePoint));
}
var UnicodePropertiesWithSpecificCase = /* @__PURE__ */ new Set([
  "Lower",
  "Lowercase",
  "Upper",
  "Uppercase",
  "Ll",
  "Lowercase_Letter",
  "Lt",
  "Titlecase_Letter",
  "Lu",
  "Uppercase_Letter"
  // The `Changes_When_*` properties (and their aliases) could be included, but they're very rare.
  // Some other properties include a handful of chars with specific cases only, but these chars are
  // generally extreme edge cases and using such properties case insensitively generally produces
  // undesired behavior anyway
]);
function transform(ast, options) {
  const opts = {
    // A couple edge cases exist where options `accuracy` and `bestEffortTarget` are used:
    // - `CharacterSet` kind `text_segment` (`\X`): An exact representation would require heavy
    //   Unicode data; a best-effort approximation requires knowing the target.
    // - `CharacterSet` kind `posix` with values `graph` and `print`: Their complex Unicode
    //   representations would be hard to change to ASCII versions after the fact in the generator
    //   based on `target`/`accuracy`, so produce the appropriate structure here.
    accuracy: "default",
    asciiWordBoundaries: false,
    avoidSubclass: false,
    bestEffortTarget: "ES2025",
    ...options
  };
  addParentProperties(ast);
  const firstPassState = {
    accuracy: opts.accuracy,
    asciiWordBoundaries: opts.asciiWordBoundaries,
    avoidSubclass: opts.avoidSubclass,
    flagDirectivesByAlt: /* @__PURE__ */ new Map(),
    jsGroupNameMap: /* @__PURE__ */ new Map(),
    minTargetEs2024: isMinTarget(opts.bestEffortTarget, "ES2024"),
    passedLookbehind: false,
    strategy: null,
    // Subroutines can appear before the groups they ref, so collect reffed nodes for a second pass 
    subroutineRefMap: /* @__PURE__ */ new Map(),
    supportedGNodes: /* @__PURE__ */ new Set(),
    digitIsAscii: ast.flags.digitIsAscii,
    spaceIsAscii: ast.flags.spaceIsAscii,
    wordIsAscii: ast.flags.wordIsAscii
  };
  S(ast, FirstPassVisitor, firstPassState);
  const globalFlags = {
    dotAll: ast.flags.dotAll,
    ignoreCase: ast.flags.ignoreCase
  };
  const secondPassState = {
    currentFlags: globalFlags,
    prevFlags: null,
    globalFlags,
    groupOriginByCopy: /* @__PURE__ */ new Map(),
    groupsByName: /* @__PURE__ */ new Map(),
    multiplexCapturesToLeftByRef: /* @__PURE__ */ new Map(),
    openRefs: /* @__PURE__ */ new Map(),
    reffedNodesByReferencer: /* @__PURE__ */ new Map(),
    subroutineRefMap: firstPassState.subroutineRefMap
  };
  S(ast, SecondPassVisitor, secondPassState);
  const thirdPassState = {
    groupsByName: secondPassState.groupsByName,
    highestOrphanBackref: 0,
    numCapturesToLeft: 0,
    reffedNodesByReferencer: secondPassState.reffedNodesByReferencer
  };
  S(ast, ThirdPassVisitor, thirdPassState);
  ast._originMap = secondPassState.groupOriginByCopy;
  ast._strategy = firstPassState.strategy;
  return ast;
}
var FirstPassVisitor = {
  AbsenceFunction({ node, parent, replaceWith }) {
    const { body: body2, kind } = node;
    if (kind === "repeater") {
      const innerGroup = A();
      innerGroup.body[0].body.push(
        // Insert own alts as `body`
        K({ negate: true, body: body2 }),
        Q("Any")
      );
      const outerGroup = A();
      outerGroup.body[0].body.push(
        _("greedy", 0, Infinity, innerGroup)
      );
      replaceWith(setParentDeep(outerGroup, parent), { traverse: true });
    } else {
      throw new Error(`Unsupported absence function "(?~|"`);
    }
  },
  Alternative: {
    enter({ node, parent, key: key2 }, { flagDirectivesByAlt }) {
      const flagDirectives = node.body.filter((el) => el.kind === "flags");
      for (let i2 = key2 + 1; i2 < parent.body.length; i2++) {
        const forwardSiblingAlt = parent.body[i2];
        getOrInsert(flagDirectivesByAlt, forwardSiblingAlt, []).push(...flagDirectives);
      }
    },
    exit({ node }, { flagDirectivesByAlt }) {
      var _a2;
      if ((_a2 = flagDirectivesByAlt.get(node)) == null ? void 0 : _a2.length) {
        const flags = getCombinedFlagModsFromFlagNodes(flagDirectivesByAlt.get(node));
        if (flags) {
          const flagGroup = A({ flags });
          flagGroup.body[0].body = node.body;
          node.body = [setParentDeep(flagGroup, node)];
        }
      }
    }
  },
  Assertion({ node, parent, key: key2, container, root: root2, remove, replaceWith }, state) {
    const { kind, negate } = node;
    const { asciiWordBoundaries, avoidSubclass, supportedGNodes, wordIsAscii } = state;
    if (kind === "text_segment_boundary") {
      throw new Error(`Unsupported text segment boundary "\\${negate ? "Y" : "y"}"`);
    } else if (kind === "line_end") {
      replaceWith(setParentDeep(K({ body: [
        b({ body: [F("string_end")] }),
        b({ body: [m(10)] })
        // `\n`
      ] }), parent));
    } else if (kind === "line_start") {
      replaceWith(setParentDeep(parseFragment(r`(?<=\A|\n(?!\z))`, { skipLookbehindValidation: true }), parent));
    } else if (kind === "search_start") {
      if (supportedGNodes.has(node)) {
        root2.flags.sticky = true;
        remove();
      } else {
        const prev = container[key2 - 1];
        if (prev && isAlwaysNonZeroLength(prev)) {
          replaceWith(setParentDeep(K({ negate: true }), parent));
        } else if (avoidSubclass) {
          throw new Error(r`Uses "\G" in a way that requires a subclass`);
        } else {
          replaceWith(setParent(F("string_start"), parent));
          state.strategy = "clip_search";
        }
      }
    } else if (kind === "string_end" || kind === "string_start") ;
    else if (kind === "string_end_newline") {
      replaceWith(setParentDeep(parseFragment(r`(?=\n?\z)`), parent));
    } else if (kind === "word_boundary") {
      if (!wordIsAscii && !asciiWordBoundaries) {
        const b2 = `(?:(?<=${defaultWordChar})(?!${defaultWordChar})|(?<!${defaultWordChar})(?=${defaultWordChar}))`;
        const B2 = `(?:(?<=${defaultWordChar})(?=${defaultWordChar})|(?<!${defaultWordChar})(?!${defaultWordChar}))`;
        replaceWith(setParentDeep(parseFragment(negate ? B2 : b2), parent));
      }
    } else {
      throw new Error(`Unexpected assertion kind "${kind}"`);
    }
  },
  Backreference({ node }, { jsGroupNameMap }) {
    let { ref } = node;
    if (typeof ref === "string" && !isValidJsGroupName(ref)) {
      ref = getAndStoreJsGroupName(ref, jsGroupNameMap);
      node.ref = ref;
    }
  },
  CapturingGroup({ node }, { jsGroupNameMap, subroutineRefMap }) {
    let { name } = node;
    if (name && !isValidJsGroupName(name)) {
      name = getAndStoreJsGroupName(name, jsGroupNameMap);
      node.name = name;
    }
    subroutineRefMap.set(node.number, node);
    if (name) {
      subroutineRefMap.set(name, node);
    }
  },
  CharacterClassRange({ node, parent, replaceWith }) {
    if (parent.kind === "intersection") {
      const cc = C({ body: [node] });
      replaceWith(setParentDeep(cc, parent), { traverse: true });
    }
  },
  CharacterSet({ node, parent, replaceWith }, { accuracy, minTargetEs2024, digitIsAscii, spaceIsAscii, wordIsAscii }) {
    const { kind, negate, value } = node;
    if (digitIsAscii && (kind === "digit" || value === "digit")) {
      replaceWith(setParent(E("digit", { negate }), parent));
      return;
    }
    if (spaceIsAscii && (kind === "space" || value === "space")) {
      replaceWith(setParentDeep(setNegate(parseFragment(asciiSpaceChar), negate), parent));
      return;
    }
    if (wordIsAscii && (kind === "word" || value === "word")) {
      replaceWith(setParent(E("word", { negate }), parent));
      return;
    }
    if (kind === "any") {
      replaceWith(setParent(Q("Any"), parent));
    } else if (kind === "digit") {
      replaceWith(setParent(Q("Nd", { negate }), parent));
    } else if (kind === "dot") ;
    else if (kind === "text_segment") {
      if (accuracy === "strict") {
        throw new Error(r`Use of "\X" requires non-strict accuracy`);
      }
      const eBase = "\\p{Emoji}(?:\\p{EMod}|\\uFE0F\\u20E3?|[\\x{E0020}-\\x{E007E}]+\\x{E007F})?";
      const emoji = r`\p{RI}{2}|${eBase}(?:\u200D${eBase})*`;
      replaceWith(setParentDeep(parseFragment(
        // Close approximation of an extended grapheme cluster; see <unicode.org/reports/tr29/>
        r`(?>\r\n|${minTargetEs2024 ? r`\p{RGI_Emoji}` : emoji}|\P{M}\p{M}*)`,
        // Allow JS property `RGI_Emoji` through
        { skipPropertyNameValidation: true }
      ), parent));
    } else if (kind === "hex") {
      replaceWith(setParent(Q("AHex", { negate }), parent));
    } else if (kind === "newline") {
      replaceWith(setParentDeep(parseFragment(negate ? "[^\n]" : "(?>\r\n?|[\n\v\f\u2028\u2029])"), parent));
    } else if (kind === "posix") {
      if (!minTargetEs2024 && (value === "graph" || value === "print")) {
        if (accuracy === "strict") {
          throw new Error(`POSIX class "${value}" requires min target ES2024 or non-strict accuracy`);
        }
        let ascii = {
          graph: "!-~",
          print: " -~"
        }[value];
        if (negate) {
          ascii = `\0-${cp(ascii.codePointAt(0) - 1)}${cp(ascii.codePointAt(2) + 1)}-􏿿`;
        }
        replaceWith(setParentDeep(parseFragment(`[${ascii}]`), parent));
      } else {
        replaceWith(setParentDeep(setNegate(parseFragment(PosixClassMap.get(value)), negate), parent));
      }
    } else if (kind === "property") {
      if (!JsUnicodePropertyMap.has(w(value))) {
        node.key = "sc";
      }
    } else if (kind === "space") {
      replaceWith(setParent(Q("space", { negate }), parent));
    } else if (kind === "word") {
      replaceWith(setParentDeep(setNegate(parseFragment(defaultWordChar), negate), parent));
    } else {
      throw new Error(`Unexpected character set kind "${kind}"`);
    }
  },
  Directive({ node, parent, root: root2, remove, replaceWith, removeAllPrevSiblings, removeAllNextSiblings }) {
    const { kind, flags } = node;
    if (kind === "flags") {
      if (!flags.enable && !flags.disable) {
        remove();
      } else {
        const flagGroup = A({ flags });
        flagGroup.body[0].body = removeAllNextSiblings();
        replaceWith(setParentDeep(flagGroup, parent), { traverse: true });
      }
    } else if (kind === "keep") {
      const firstAlt = root2.body[0];
      const hasWrapperGroup = root2.body.length === 1 && // Not emulatable if within a `CapturingGroup`
      o(firstAlt, { type: "Group" }) && firstAlt.body[0].body.length === 1;
      const topLevel = hasWrapperGroup ? firstAlt.body[0] : root2;
      if (parent.parent !== topLevel || topLevel.body.length > 1) {
        throw new Error(r`Uses "\K" in a way that's unsupported`);
      }
      const lookbehind = K({ behind: true });
      lookbehind.body[0].body = removeAllPrevSiblings();
      replaceWith(setParentDeep(lookbehind, parent));
    } else {
      throw new Error(`Unexpected directive kind "${kind}"`);
    }
  },
  Flags({ node, parent }) {
    if (node.posixIsAscii) {
      throw new Error('Unsupported flag "P"');
    }
    if (node.textSegmentMode === "word") {
      throw new Error('Unsupported flag "y{w}"');
    }
    [
      "digitIsAscii",
      // Flag D
      "extended",
      // Flag x
      "posixIsAscii",
      // Flag P
      "spaceIsAscii",
      // Flag S
      "wordIsAscii",
      // Flag W
      "textSegmentMode"
      // Flag y{g} or y{w}
    ].forEach((f2) => delete node[f2]);
    Object.assign(node, {
      // JS flag g; no Onig equiv
      global: false,
      // JS flag d; no Onig equiv
      hasIndices: false,
      // JS flag m; no Onig equiv but its behavior is always on in Onig. Onig's only line break
      // char is line feed, unlike JS, so this flag isn't used since it would produce inaccurate
      // results (also allows `^` and `$` to be used in the generator for string start and end)
      multiline: false,
      // JS flag y; no Onig equiv, but used for `\G` emulation
      sticky: node.sticky ?? false
      // Note: Regex+ doesn't allow explicitly adding flags it handles implicitly, so leave out
      // properties `unicode` (JS flag u) and `unicodeSets` (JS flag v). Keep the existing values
      // for `ignoreCase` (flag i) and `dotAll` (JS flag s, but Onig flag m)
    });
    parent.options = {
      disable: {
        // Onig uses different rules for flag x than Regex+, so disable the implicit flag
        x: true,
        // Onig has no flag to control "named capture only" mode but contextually applies its
        // behavior when named capturing is used, so disable Regex+'s implicit flag for it
        n: true
      },
      force: {
        // Always add flag v because we're generating an AST that relies on it (it enables JS
        // support for Onig features nested classes, intersection, Unicode properties, etc.).
        // However, the generator might disable flag v based on its `target` option
        v: true
      }
    };
  },
  Group({ node }) {
    if (!node.flags) {
      return;
    }
    const { enable, disable } = node.flags;
    (enable == null ? void 0 : enable.extended) && delete enable.extended;
    (disable == null ? void 0 : disable.extended) && delete disable.extended;
    (enable == null ? void 0 : enable.dotAll) && (disable == null ? void 0 : disable.dotAll) && delete enable.dotAll;
    (enable == null ? void 0 : enable.ignoreCase) && (disable == null ? void 0 : disable.ignoreCase) && delete enable.ignoreCase;
    enable && !Object.keys(enable).length && delete node.flags.enable;
    disable && !Object.keys(disable).length && delete node.flags.disable;
    !node.flags.enable && !node.flags.disable && delete node.flags;
  },
  LookaroundAssertion({ node }, state) {
    const { kind } = node;
    if (kind === "lookbehind") {
      state.passedLookbehind = true;
    }
  },
  NamedCallout({ node, parent, replaceWith }) {
    const { kind } = node;
    if (kind === "fail") {
      replaceWith(setParentDeep(K({ negate: true }), parent));
    } else {
      throw new Error(`Unsupported named callout "(*${kind.toUpperCase()}"`);
    }
  },
  Quantifier({ node }) {
    if (node.body.type === "Quantifier") {
      const group = A();
      group.body[0].body.push(node.body);
      node.body = setParentDeep(group, node);
    }
  },
  Regex: {
    enter({ node }, { supportedGNodes }) {
      const leadingGs = [];
      let hasAltWithLeadG = false;
      let hasAltWithoutLeadG = false;
      for (const alt of node.body) {
        if (alt.body.length === 1 && alt.body[0].kind === "search_start") {
          alt.body.pop();
        } else {
          const leadingG = getLeadingG(alt.body);
          if (leadingG) {
            hasAltWithLeadG = true;
            Array.isArray(leadingG) ? leadingGs.push(...leadingG) : leadingGs.push(leadingG);
          } else {
            hasAltWithoutLeadG = true;
          }
        }
      }
      if (hasAltWithLeadG && !hasAltWithoutLeadG) {
        leadingGs.forEach((g) => supportedGNodes.add(g));
      }
    },
    exit(_2, { accuracy, passedLookbehind, strategy }) {
      if (accuracy === "strict" && passedLookbehind && strategy) {
        throw new Error(r`Uses "\G" in a way that requires non-strict accuracy`);
      }
    }
  },
  Subroutine({ node }, { jsGroupNameMap }) {
    let { ref } = node;
    if (typeof ref === "string" && !isValidJsGroupName(ref)) {
      ref = getAndStoreJsGroupName(ref, jsGroupNameMap);
      node.ref = ref;
    }
  }
};
var SecondPassVisitor = {
  Backreference({ node }, { multiplexCapturesToLeftByRef, reffedNodesByReferencer }) {
    const { orphan, ref } = node;
    if (!orphan) {
      reffedNodesByReferencer.set(node, [...multiplexCapturesToLeftByRef.get(ref).map(({ node: node2 }) => node2)]);
    }
  },
  CapturingGroup: {
    enter({
      node,
      parent,
      replaceWith,
      skip
    }, {
      groupOriginByCopy,
      groupsByName,
      multiplexCapturesToLeftByRef,
      openRefs,
      reffedNodesByReferencer
    }) {
      const origin = groupOriginByCopy.get(node);
      if (origin && openRefs.has(node.number)) {
        const recursion2 = setParent(createRecursion(node.number), parent);
        reffedNodesByReferencer.set(recursion2, openRefs.get(node.number));
        replaceWith(recursion2);
        return;
      }
      openRefs.set(node.number, node);
      multiplexCapturesToLeftByRef.set(node.number, []);
      if (node.name) {
        getOrInsert(multiplexCapturesToLeftByRef, node.name, []);
      }
      const multiplexNodes = multiplexCapturesToLeftByRef.get(node.name ?? node.number);
      for (let i2 = 0; i2 < multiplexNodes.length; i2++) {
        const multiplex = multiplexNodes[i2];
        if (
          // This group is from subroutine expansion, and there's a multiplex value from either the
          // origin node or a prior subroutine expansion group with the same origin
          origin === multiplex.node || origin && origin === multiplex.origin || // This group is not from subroutine expansion, and it comes after a subroutine expansion
          // group that refers to this group
          node === multiplex.origin
        ) {
          multiplexNodes.splice(i2, 1);
          break;
        }
      }
      multiplexCapturesToLeftByRef.get(node.number).push({ node, origin });
      if (node.name) {
        multiplexCapturesToLeftByRef.get(node.name).push({ node, origin });
      }
      if (node.name) {
        const groupsWithSameName = getOrInsert(groupsByName, node.name, /* @__PURE__ */ new Map());
        let hasDuplicateNameToRemove = false;
        if (origin) {
          hasDuplicateNameToRemove = true;
        } else {
          for (const groupInfo of groupsWithSameName.values()) {
            if (!groupInfo.hasDuplicateNameToRemove) {
              hasDuplicateNameToRemove = true;
              break;
            }
          }
        }
        groupsByName.get(node.name).set(node, { node, hasDuplicateNameToRemove });
      }
    },
    exit({ node }, { openRefs }) {
      openRefs.delete(node.number);
    }
  },
  Group: {
    enter({ node }, state) {
      state.prevFlags = state.currentFlags;
      if (node.flags) {
        state.currentFlags = getNewCurrentFlags(state.currentFlags, node.flags);
      }
    },
    exit(_2, state) {
      state.currentFlags = state.prevFlags;
    }
  },
  Subroutine({ node, parent, replaceWith }, state) {
    const { isRecursive, ref } = node;
    if (isRecursive) {
      let reffed = parent;
      while (reffed = reffed.parent) {
        if (reffed.type === "CapturingGroup" && (reffed.name === ref || reffed.number === ref)) {
          break;
        }
      }
      state.reffedNodesByReferencer.set(node, reffed);
      return;
    }
    const reffedGroupNode = state.subroutineRefMap.get(ref);
    const isGlobalRecursion = ref === 0;
    const expandedSubroutine = isGlobalRecursion ? createRecursion(0) : (
      // The reffed group might itself contain subroutines, which are expanded during sub-traversal
      cloneCapturingGroup(reffedGroupNode, state.groupOriginByCopy, null)
    );
    let replacement = expandedSubroutine;
    if (!isGlobalRecursion) {
      const reffedGroupFlagMods = getCombinedFlagModsFromFlagNodes(getAllParents(
        reffedGroupNode,
        (p2) => p2.type === "Group" && !!p2.flags
      ));
      const reffedGroupFlags = reffedGroupFlagMods ? getNewCurrentFlags(state.globalFlags, reffedGroupFlagMods) : state.globalFlags;
      if (!areFlagsEqual(reffedGroupFlags, state.currentFlags)) {
        replacement = A({
          flags: getFlagModsFromFlags(reffedGroupFlags)
        });
        replacement.body[0].body.push(expandedSubroutine);
      }
    }
    replaceWith(setParentDeep(replacement, parent), { traverse: !isGlobalRecursion });
  }
};
var ThirdPassVisitor = {
  Backreference({ node, parent, replaceWith }, state) {
    if (node.orphan) {
      state.highestOrphanBackref = Math.max(state.highestOrphanBackref, node.ref);
      return;
    }
    const reffedNodes = state.reffedNodesByReferencer.get(node);
    const participants = reffedNodes.filter((reffed) => canParticipateWithNode(reffed, node));
    if (!participants.length) {
      replaceWith(setParentDeep(K({ negate: true }), parent));
    } else if (participants.length > 1) {
      const group = A({
        atomic: true,
        body: participants.reverse().map((reffed) => b({
          body: [k(reffed.number)]
        }))
      });
      replaceWith(setParentDeep(group, parent));
    } else {
      node.ref = participants[0].number;
    }
  },
  CapturingGroup({ node }, state) {
    node.number = ++state.numCapturesToLeft;
    if (node.name) {
      if (state.groupsByName.get(node.name).get(node).hasDuplicateNameToRemove) {
        delete node.name;
      }
    }
  },
  Regex: {
    exit({ node }, state) {
      const numCapsNeeded = Math.max(state.highestOrphanBackref - state.numCapturesToLeft, 0);
      for (let i2 = 0; i2 < numCapsNeeded; i2++) {
        const emptyCapture = P();
        node.body.at(-1).body.push(emptyCapture);
      }
    }
  },
  Subroutine({ node }, state) {
    if (!node.isRecursive || node.ref === 0) {
      return;
    }
    node.ref = state.reffedNodesByReferencer.get(node).number;
  }
};
function addParentProperties(root2) {
  S(root2, {
    "*"({ node, parent }) {
      node.parent = parent;
    }
  });
}
function areFlagsEqual(a, b2) {
  return a.dotAll === b2.dotAll && a.ignoreCase === b2.ignoreCase;
}
function canParticipateWithNode(capture, node) {
  let rightmostPoint = node;
  do {
    if (rightmostPoint.type === "Regex") {
      return false;
    }
    if (rightmostPoint.type === "Alternative") {
      continue;
    }
    if (rightmostPoint === capture) {
      return false;
    }
    const kidsOfParent = getKids(rightmostPoint.parent);
    for (const kid of kidsOfParent) {
      if (kid === rightmostPoint) {
        break;
      }
      if (kid === capture || isAncestorOf(kid, capture)) {
        return true;
      }
    }
  } while (rightmostPoint = rightmostPoint.parent);
  throw new Error("Unexpected path");
}
function cloneCapturingGroup(obj, originMap, up, up2) {
  const store = Array.isArray(obj) ? [] : {};
  for (const [key2, value] of Object.entries(obj)) {
    if (key2 === "parent") {
      store.parent = Array.isArray(up) ? up2 : up;
    } else if (value && typeof value === "object") {
      store[key2] = cloneCapturingGroup(value, originMap, store, up);
    } else {
      if (key2 === "type" && value === "CapturingGroup") {
        originMap.set(store, originMap.get(obj) ?? obj);
      }
      store[key2] = value;
    }
  }
  return store;
}
function createRecursion(ref) {
  const node = O(ref);
  node.isRecursive = true;
  return node;
}
function getAllParents(node, filterFn) {
  const results = [];
  while (node = node.parent) {
    if (!filterFn || filterFn(node)) {
      results.push(node);
    }
  }
  return results;
}
function getAndStoreJsGroupName(name, map) {
  if (map.has(name)) {
    return map.get(name);
  }
  const jsName = `$${map.size}_${name.replace(/^[^$_\p{IDS}]|[^$\u200C\u200D\p{IDC}]/ug, "_")}`;
  map.set(name, jsName);
  return jsName;
}
function getCombinedFlagModsFromFlagNodes(flagNodes) {
  const flagProps = ["dotAll", "ignoreCase"];
  const combinedFlags = { enable: {}, disable: {} };
  flagNodes.forEach(({ flags }) => {
    flagProps.forEach((prop) => {
      var _a2, _b2;
      if ((_a2 = flags.enable) == null ? void 0 : _a2[prop]) {
        delete combinedFlags.disable[prop];
        combinedFlags.enable[prop] = true;
      }
      if ((_b2 = flags.disable) == null ? void 0 : _b2[prop]) {
        combinedFlags.disable[prop] = true;
      }
    });
  });
  if (!Object.keys(combinedFlags.enable).length) {
    delete combinedFlags.enable;
  }
  if (!Object.keys(combinedFlags.disable).length) {
    delete combinedFlags.disable;
  }
  if (combinedFlags.enable || combinedFlags.disable) {
    return combinedFlags;
  }
  return null;
}
function getFlagModsFromFlags({ dotAll, ignoreCase }) {
  const mods = {};
  if (dotAll || ignoreCase) {
    mods.enable = {};
    dotAll && (mods.enable.dotAll = true);
    ignoreCase && (mods.enable.ignoreCase = true);
  }
  if (!dotAll || !ignoreCase) {
    mods.disable = {};
    !dotAll && (mods.disable.dotAll = true);
    !ignoreCase && (mods.disable.ignoreCase = true);
  }
  return mods;
}
function getKids(node) {
  if (!node) {
    throw new Error("Node expected");
  }
  const { body: body2 } = node;
  return Array.isArray(body2) ? body2 : body2 ? [body2] : null;
}
function getLeadingG(els) {
  const firstToConsider = els.find((el) => el.kind === "search_start" || isLoneGLookaround(el, { negate: false }) || !isAlwaysZeroLength(el));
  if (!firstToConsider) {
    return null;
  }
  if (firstToConsider.kind === "search_start") {
    return firstToConsider;
  }
  if (firstToConsider.type === "LookaroundAssertion") {
    return firstToConsider.body[0].body[0];
  }
  if (firstToConsider.type === "CapturingGroup" || firstToConsider.type === "Group") {
    const gNodesForGroup = [];
    for (const alt of firstToConsider.body) {
      const leadingG = getLeadingG(alt.body);
      if (!leadingG) {
        return null;
      }
      Array.isArray(leadingG) ? gNodesForGroup.push(...leadingG) : gNodesForGroup.push(leadingG);
    }
    return gNodesForGroup;
  }
  return null;
}
function isAncestorOf(node, descendant) {
  const kids = getKids(node) ?? [];
  for (const kid of kids) {
    if (kid === descendant || isAncestorOf(kid, descendant)) {
      return true;
    }
  }
  return false;
}
function isAlwaysZeroLength({ type }) {
  return type === "Assertion" || type === "Directive" || type === "LookaroundAssertion";
}
function isAlwaysNonZeroLength(node) {
  const types2 = [
    "Character",
    "CharacterClass",
    "CharacterSet"
  ];
  return types2.includes(node.type) || node.type === "Quantifier" && node.min && types2.includes(node.body.type);
}
function isLoneGLookaround(node, options) {
  const opts = {
    negate: null,
    ...options
  };
  return node.type === "LookaroundAssertion" && (opts.negate === null || node.negate === opts.negate) && node.body.length === 1 && o(node.body[0], {
    type: "Assertion",
    kind: "search_start"
  });
}
function isValidJsGroupName(name) {
  return /^[$_\p{IDS}][$\u200C\u200D\p{IDC}]*$/u.test(name);
}
function parseFragment(pattern, options) {
  const ast = J(pattern, {
    ...options,
    // Providing a custom set of Unicode property names avoids converting some JS Unicode
    // properties (ex: `\p{Alpha}`) to Onig POSIX classes
    unicodePropertyMap: JsUnicodePropertyMap
  });
  const alts = ast.body;
  if (alts.length > 1 || alts[0].body.length > 1) {
    return A({ body: alts });
  }
  return alts[0].body[0];
}
function setNegate(node, negate) {
  node.negate = negate;
  return node;
}
function setParent(node, parent) {
  node.parent = parent;
  return node;
}
function setParentDeep(node, parent) {
  addParentProperties(node);
  node.parent = parent;
  return node;
}
function generate(ast, options) {
  const opts = getOptions(options);
  const minTargetEs2024 = isMinTarget(opts.target, "ES2024");
  const minTargetEs2025 = isMinTarget(opts.target, "ES2025");
  const recursionLimit = opts.rules.recursionLimit;
  if (!Number.isInteger(recursionLimit) || recursionLimit < 2 || recursionLimit > 20) {
    throw new Error("Invalid recursionLimit; use 2-20");
  }
  let hasCaseInsensitiveNode = null;
  let hasCaseSensitiveNode = null;
  if (!minTargetEs2025) {
    const iStack = [ast.flags.ignoreCase];
    S(ast, FlagModifierVisitor, {
      getCurrentModI: () => iStack.at(-1),
      popModI() {
        iStack.pop();
      },
      pushModI(isIOn) {
        iStack.push(isIOn);
      },
      setHasCasedChar() {
        if (iStack.at(-1)) {
          hasCaseInsensitiveNode = true;
        } else {
          hasCaseSensitiveNode = true;
        }
      }
    });
  }
  const appliedGlobalFlags = {
    dotAll: ast.flags.dotAll,
    // - Turn global flag i on if a case insensitive node was used and no case sensitive nodes were
    //   used (to avoid unnecessary node expansion).
    // - Turn global flag i off if a case sensitive node was used (since case sensitivity can't be
    //   forced without the use of ES2025 flag groups)
    ignoreCase: !!((ast.flags.ignoreCase || hasCaseInsensitiveNode) && !hasCaseSensitiveNode)
  };
  let lastNode = ast;
  const state = {
    accuracy: opts.accuracy,
    appliedGlobalFlags,
    captureMap: /* @__PURE__ */ new Map(),
    currentFlags: {
      dotAll: ast.flags.dotAll,
      ignoreCase: ast.flags.ignoreCase
    },
    inCharClass: false,
    lastNode,
    originMap: ast._originMap,
    recursionLimit,
    useAppliedIgnoreCase: !!(!minTargetEs2025 && hasCaseInsensitiveNode && hasCaseSensitiveNode),
    useFlagMods: minTargetEs2025,
    useFlagV: minTargetEs2024,
    verbose: opts.verbose
  };
  function gen(node) {
    state.lastNode = lastNode;
    lastNode = node;
    const fn = throwIfNullish(generator[node.type], `Unexpected node type "${node.type}"`);
    return fn(node, state, gen);
  }
  const result = {
    pattern: ast.body.map(gen).join("|"),
    // Could reset `lastNode` at this point via `lastNode = ast`, but it isn't needed by flags
    flags: gen(ast.flags),
    options: { ...ast.options }
  };
  if (!minTargetEs2024) {
    delete result.options.force.v;
    result.options.disable.v = true;
    result.options.unicodeSetsPlugin = null;
  }
  result._captureTransfers = /* @__PURE__ */ new Map();
  result._hiddenCaptures = [];
  state.captureMap.forEach((value, key2) => {
    if (value.hidden) {
      result._hiddenCaptures.push(key2);
    }
    if (value.transferTo) {
      getOrInsert(result._captureTransfers, value.transferTo, []).push(key2);
    }
  });
  return result;
}
var FlagModifierVisitor = {
  "*": {
    enter({ node }, state) {
      if (isAnyGroup(node)) {
        const currentModI = state.getCurrentModI();
        state.pushModI(
          node.flags ? getNewCurrentFlags({ ignoreCase: currentModI }, node.flags).ignoreCase : currentModI
        );
      }
    },
    exit({ node }, state) {
      if (isAnyGroup(node)) {
        state.popModI();
      }
    }
  },
  Backreference(_2, state) {
    state.setHasCasedChar();
  },
  Character({ node }, state) {
    if (charHasCase(cp(node.value))) {
      state.setHasCasedChar();
    }
  },
  CharacterClassRange({ node, skip }, state) {
    skip();
    if (getCasesOutsideCharClassRange(node, { firstOnly: true }).length) {
      state.setHasCasedChar();
    }
  },
  CharacterSet({ node }, state) {
    if (node.kind === "property" && UnicodePropertiesWithSpecificCase.has(node.value)) {
      state.setHasCasedChar();
    }
  }
};
var generator = {
  /**
  @param {AlternativeNode} node
  */
  Alternative({ body: body2 }, _2, gen) {
    return body2.map(gen).join("");
  },
  /**
  @param {AssertionNode} node
  */
  Assertion({ kind, negate }) {
    if (kind === "string_end") {
      return "$";
    }
    if (kind === "string_start") {
      return "^";
    }
    if (kind === "word_boundary") {
      return negate ? r`\B` : r`\b`;
    }
    throw new Error(`Unexpected assertion kind "${kind}"`);
  },
  /**
  @param {BackreferenceNode} node
  */
  Backreference({ ref }, state) {
    if (typeof ref !== "number") {
      throw new Error("Unexpected named backref in transformed AST");
    }
    if (!state.useFlagMods && state.accuracy === "strict" && state.currentFlags.ignoreCase && !state.captureMap.get(ref).ignoreCase) {
      throw new Error("Use of case-insensitive backref to case-sensitive group requires target ES2025 or non-strict accuracy");
    }
    return "\\" + ref;
  },
  /**
  @param {CapturingGroupNode} node
  */
  CapturingGroup(node, state, gen) {
    const { body: body2, name, number: number2 } = node;
    const data = { ignoreCase: state.currentFlags.ignoreCase };
    const origin = state.originMap.get(node);
    if (origin) {
      data.hidden = true;
      if (number2 > origin.number) {
        data.transferTo = origin.number;
      }
    }
    state.captureMap.set(number2, data);
    return `(${name ? `?<${name}>` : ""}${body2.map(gen).join("|")})`;
  },
  /**
  @param {CharacterNode} node
  */
  Character({ value }, state) {
    const char = cp(value);
    const escaped = getCharEscape(value, {
      escDigit: state.lastNode.type === "Backreference",
      inCharClass: state.inCharClass,
      useFlagV: state.useFlagV
    });
    if (escaped !== char) {
      return escaped;
    }
    if (state.useAppliedIgnoreCase && state.currentFlags.ignoreCase && charHasCase(char)) {
      const cases = getIgnoreCaseMatchChars(char);
      return state.inCharClass ? cases.join("") : cases.length > 1 ? `[${cases.join("")}]` : cases[0];
    }
    return char;
  },
  /**
  @param {CharacterClassNode} node
  */
  CharacterClass(node, state, gen) {
    const { kind, negate, parent } = node;
    let { body: body2 } = node;
    if (kind === "intersection" && !state.useFlagV) {
      throw new Error("Use of character class intersection requires min target ES2024");
    }
    if (envFlags.bugFlagVLiteralHyphenIsRange && state.useFlagV && body2.some(isLiteralHyphen)) {
      body2 = [m(45), ...body2.filter((kid) => !isLiteralHyphen(kid))];
    }
    const genClass = () => `[${negate ? "^" : ""}${body2.map(gen).join(kind === "intersection" ? "&&" : "")}]`;
    if (!state.inCharClass) {
      if (
        // Already established `kind !== 'intersection'` if `!state.useFlagV`; don't check again
        (!state.useFlagV || envFlags.bugNestedClassIgnoresNegation) && !negate
      ) {
        const negatedChildClasses = body2.filter(
          (kid) => kid.type === "CharacterClass" && kid.kind === "union" && kid.negate
        );
        if (negatedChildClasses.length) {
          const group = A();
          const groupFirstAlt = group.body[0];
          group.parent = parent;
          groupFirstAlt.parent = group;
          body2 = body2.filter((kid) => !negatedChildClasses.includes(kid));
          node.body = body2;
          if (body2.length) {
            node.parent = groupFirstAlt;
            groupFirstAlt.body.push(node);
          } else {
            group.body.pop();
          }
          negatedChildClasses.forEach((cc) => {
            const newAlt = b({ body: [cc] });
            cc.parent = newAlt;
            newAlt.parent = group;
            group.body.push(newAlt);
          });
          return gen(group);
        }
      }
      state.inCharClass = true;
      const result = genClass();
      state.inCharClass = false;
      return result;
    }
    const firstEl = body2[0];
    if (
      // Already established that the parent is a char class via `inCharClass`; don't check again
      kind === "union" && !negate && firstEl && // Allows many nested classes to work with `target` ES2018 which doesn't support nesting
      ((!state.useFlagV || !state.verbose) && parent.kind === "union" && !(envFlags.bugFlagVLiteralHyphenIsRange && state.useFlagV) || !state.verbose && parent.kind === "intersection" && // JS doesn't allow intersection with union or ranges
      body2.length === 1 && firstEl.type !== "CharacterClassRange")
    ) {
      return body2.map(gen).join("");
    }
    if (!state.useFlagV && parent.type === "CharacterClass") {
      throw new Error("Uses nested character class in a way that requires min target ES2024");
    }
    return genClass();
  },
  /**
  @param {CharacterClassRangeNode} node
  */
  CharacterClassRange(node, state) {
    const min = node.min.value;
    const max = node.max.value;
    const escOpts = {
      escDigit: false,
      inCharClass: true,
      useFlagV: state.useFlagV
    };
    const minStr = getCharEscape(min, escOpts);
    const maxStr = getCharEscape(max, escOpts);
    const extraChars = /* @__PURE__ */ new Set();
    if (state.useAppliedIgnoreCase && state.currentFlags.ignoreCase) {
      const charsOutsideRange = getCasesOutsideCharClassRange(node);
      const ranges = getCodePointRangesFromChars(charsOutsideRange);
      ranges.forEach((value) => {
        extraChars.add(
          Array.isArray(value) ? `${getCharEscape(value[0], escOpts)}-${getCharEscape(value[1], escOpts)}` : getCharEscape(value, escOpts)
        );
      });
    }
    return `${minStr}-${maxStr}${[...extraChars].join("")}`;
  },
  /**
  @param {CharacterSetNode} node
  */
  CharacterSet({ kind, negate, value, key: key2 }, state) {
    if (kind === "dot") {
      return state.currentFlags.dotAll ? state.appliedGlobalFlags.dotAll || state.useFlagMods ? "." : "[^]" : (
        // Onig's only line break char is line feed, unlike JS
        r`[^\n]`
      );
    }
    if (kind === "digit") {
      return negate ? r`\D` : r`\d`;
    }
    if (kind === "property") {
      if (state.useAppliedIgnoreCase && state.currentFlags.ignoreCase && UnicodePropertiesWithSpecificCase.has(value)) {
        throw new Error(`Unicode property "${value}" can't be case-insensitive when other chars have specific case`);
      }
      return `${negate ? r`\P` : r`\p`}{${key2 ? `${key2}=` : ""}${value}}`;
    }
    if (kind === "word") {
      return negate ? r`\W` : r`\w`;
    }
    throw new Error(`Unexpected character set kind "${kind}"`);
  },
  /**
  @param {FlagsNode} node
  */
  Flags(node, state) {
    return (
      // The transformer should never turn on the properties for flags d, g, m since Onig doesn't
      // have equivs. Flag m is never used since Onig uses different line break chars than JS
      // (node.hasIndices ? 'd' : '') +
      // (node.global ? 'g' : '') +
      // (node.multiline ? 'm' : '') +
      (state.appliedGlobalFlags.ignoreCase ? "i" : "") + (node.dotAll ? "s" : "") + (node.sticky ? "y" : "")
    );
  },
  /**
  @param {GroupNode} node
  */
  Group({ atomic: atomic2, body: body2, flags, parent }, state, gen) {
    const currentFlags = state.currentFlags;
    if (flags) {
      state.currentFlags = getNewCurrentFlags(currentFlags, flags);
    }
    const contents = body2.map(gen).join("|");
    const result = !state.verbose && body2.length === 1 && // Single alt
    parent.type !== "Quantifier" && !atomic2 && (!state.useFlagMods || !flags) ? contents : `(?${getGroupPrefix(atomic2, flags, state.useFlagMods)}${contents})`;
    state.currentFlags = currentFlags;
    return result;
  },
  /**
  @param {LookaroundAssertionNode} node
  */
  LookaroundAssertion({ body: body2, kind, negate }, _2, gen) {
    const prefix = `${kind === "lookahead" ? "" : "<"}${negate ? "!" : "="}`;
    return `(?${prefix}${body2.map(gen).join("|")})`;
  },
  /**
  @param {QuantifierNode} node
  */
  Quantifier(node, _2, gen) {
    return gen(node.body) + getQuantifierStr(node);
  },
  /**
  @param {SubroutineNode & {isRecursive: true}} node
  */
  Subroutine({ isRecursive, ref }, state) {
    if (!isRecursive) {
      throw new Error("Unexpected non-recursive subroutine in transformed AST");
    }
    const limit = state.recursionLimit;
    return ref === 0 ? `(?R=${limit})` : r`\g<${ref}&R=${limit}>`;
  }
};
var BaseEscapeChars = /* @__PURE__ */ new Set([
  "$",
  "(",
  ")",
  "*",
  "+",
  ".",
  "?",
  "[",
  "\\",
  "]",
  "^",
  "{",
  "|",
  "}"
]);
var CharClassEscapeChars = /* @__PURE__ */ new Set([
  "-",
  "\\",
  "]",
  "^",
  // Literal `[` doesn't require escaping with flag u, but this can help work around regex source
  // linters and regex syntax processors that expect unescaped `[` to create a nested class
  "["
]);
var CharClassEscapeCharsFlagV = /* @__PURE__ */ new Set([
  "(",
  ")",
  "-",
  "/",
  "[",
  "\\",
  "]",
  "^",
  "{",
  "|",
  "}",
  // Double punctuators; also includes already-listed `-` and `^`
  "!",
  "#",
  "$",
  "%",
  "&",
  "*",
  "+",
  ",",
  ".",
  ":",
  ";",
  "<",
  "=",
  ">",
  "?",
  "@",
  "`",
  "~"
]);
var CharCodeEscapeMap = /* @__PURE__ */ new Map([
  [9, r`\t`],
  // horizontal tab
  [10, r`\n`],
  // line feed
  [11, r`\v`],
  // vertical tab
  [12, r`\f`],
  // form feed
  [13, r`\r`],
  // carriage return
  [8232, r`\u2028`],
  // line separator
  [8233, r`\u2029`],
  // paragraph separator
  [65279, r`\uFEFF`]
  // ZWNBSP/BOM
]);
var casedRe = new RegExp("^\\p{Cased}$", "u");
function charHasCase(char) {
  return casedRe.test(char);
}
function getCasesOutsideCharClassRange(node, options) {
  const firstOnly = !!(options == null ? void 0 : options.firstOnly);
  const min = node.min.value;
  const max = node.max.value;
  const found = [];
  if (min < 65 && (max === 65535 || max >= 131071) || min === 65536 && max >= 131071) {
    return found;
  }
  for (let i2 = min; i2 <= max; i2++) {
    const char = cp(i2);
    if (!charHasCase(char)) {
      continue;
    }
    const charsOutsideRange = getIgnoreCaseMatchChars(char).filter((caseOfChar) => {
      const num = caseOfChar.codePointAt(0);
      return num < min || num > max;
    });
    if (charsOutsideRange.length) {
      found.push(...charsOutsideRange);
      if (firstOnly) {
        break;
      }
    }
  }
  return found;
}
function getCharEscape(codePoint, { escDigit, inCharClass, useFlagV }) {
  if (CharCodeEscapeMap.has(codePoint)) {
    return CharCodeEscapeMap.get(codePoint);
  }
  if (
    // Control chars, etc.; condition modeled on the Chrome developer console's display for strings
    codePoint < 32 || codePoint > 126 && codePoint < 160 || // Unicode planes 4-16; unassigned, special purpose, and private use area
    codePoint > 262143 || // Avoid corrupting a preceding backref by immediately following it with a literal digit
    escDigit && isDigitCharCode(codePoint)
  ) {
    return codePoint > 255 ? `\\u{${codePoint.toString(16).toUpperCase()}}` : `\\x${codePoint.toString(16).toUpperCase().padStart(2, "0")}`;
  }
  const escapeChars = inCharClass ? useFlagV ? CharClassEscapeCharsFlagV : CharClassEscapeChars : BaseEscapeChars;
  const char = cp(codePoint);
  return (escapeChars.has(char) ? "\\" : "") + char;
}
function getCodePointRangesFromChars(chars) {
  const codePoints = chars.map((char) => char.codePointAt(0)).sort((a, b2) => a - b2);
  const values = [];
  let start = null;
  for (let i2 = 0; i2 < codePoints.length; i2++) {
    if (codePoints[i2 + 1] === codePoints[i2] + 1) {
      start ?? (start = codePoints[i2]);
    } else if (start === null) {
      values.push(codePoints[i2]);
    } else {
      values.push([start, codePoints[i2]]);
      start = null;
    }
  }
  return values;
}
function getGroupPrefix(atomic2, flagMods, useFlagMods) {
  if (atomic2) {
    return ">";
  }
  let mods = "";
  if (flagMods && useFlagMods) {
    const { enable, disable } = flagMods;
    mods = ((enable == null ? void 0 : enable.ignoreCase) ? "i" : "") + ((enable == null ? void 0 : enable.dotAll) ? "s" : "") + (disable ? "-" : "") + ((disable == null ? void 0 : disable.ignoreCase) ? "i" : "") + ((disable == null ? void 0 : disable.dotAll) ? "s" : "");
  }
  return `${mods}:`;
}
function getQuantifierStr({ kind, max, min }) {
  let base;
  if (!min && max === 1) {
    base = "?";
  } else if (!min && max === Infinity) {
    base = "*";
  } else if (min === 1 && max === Infinity) {
    base = "+";
  } else if (min === max) {
    base = `{${min}}`;
  } else {
    base = `{${min},${max === Infinity ? "" : max}}`;
  }
  return base + {
    greedy: "",
    lazy: "?",
    possessive: "+"
  }[kind];
}
function isAnyGroup({ type }) {
  return type === "CapturingGroup" || type === "Group" || type === "LookaroundAssertion";
}
function isDigitCharCode(value) {
  return value > 47 && value < 58;
}
function isLiteralHyphen({ type, value }) {
  return type === "Character" && value === 45;
}
var EmulatedRegExp = (_c = class extends RegExp {
  /**
  @overload
  @param {string} pattern
  @param {string} [flags]
  @param {EmulatedRegExpOptions} [options]
  */
  /**
  @overload
  @param {EmulatedRegExp} pattern
  @param {string} [flags]
  */
  constructor(pattern, flags, options) {
    var __super = (...args) => {
      super(...args);
      __privateAdd(this, __EmulatedRegExp_instances);
      /**
      @type {Map<number, {
        hidden?: true;
        transferTo?: number;
      }>}
      */
      __privateAdd(this, _captureMap, /* @__PURE__ */ new Map());
      /**
      @type {RegExp | EmulatedRegExp | null}
      */
      __privateAdd(this, _compiled, null);
      /**
      @type {string}
      */
      __privateAdd(this, _pattern);
      /**
      @type {Map<number, string>?}
      */
      __privateAdd(this, _nameMap, null);
      /**
      @type {string?}
      */
      __privateAdd(this, _strategy, null);
      /**
      Can be used to serialize the instance.
      @type {EmulatedRegExpOptions}
      */
      __publicField(this, "rawOptions", {});
      return this;
    };
    const lazyCompile = !!(options == null ? void 0 : options.lazyCompile);
    if (pattern instanceof RegExp) {
      if (options) {
        throw new Error("Cannot provide options when copying a regexp");
      }
      const re2 = pattern;
      __super(re2, flags);
      __privateSet(this, _pattern, re2.source);
      if (re2 instanceof _c) {
        __privateSet(this, _captureMap, __privateGet(re2, _captureMap));
        __privateSet(this, _nameMap, __privateGet(re2, _nameMap));
        __privateSet(this, _strategy, __privateGet(re2, _strategy));
        this.rawOptions = re2.rawOptions;
      }
    } else {
      const opts = {
        hiddenCaptures: [],
        strategy: null,
        transfers: [],
        ...options
      };
      __super(lazyCompile ? "" : pattern, flags);
      __privateSet(this, _pattern, pattern);
      __privateSet(this, _captureMap, createCaptureMap(opts.hiddenCaptures, opts.transfers));
      __privateSet(this, _strategy, opts.strategy);
      this.rawOptions = options ?? {};
    }
    if (!lazyCompile) {
      __privateSet(this, _compiled, this);
    }
  }
  // Override the getter with one that works with lazy-compiled regexes
  get source() {
    return __privateGet(this, _pattern) || "(?:)";
  }
  /**
  Called internally by all String/RegExp methods that use regexes.
  @override
  @param {string} str
  @returns {RegExpExecArray?}
  */
  exec(str) {
    if (!__privateGet(this, _compiled)) {
      const { lazyCompile, ...rest } = this.rawOptions;
      __privateSet(this, _compiled, new _c(__privateGet(this, _pattern), this.flags, rest));
    }
    const useLastIndex = this.global || this.sticky;
    const pos = this.lastIndex;
    if (__privateGet(this, _strategy) === "clip_search" && useLastIndex && pos) {
      this.lastIndex = 0;
      const match = __privateMethod(this, __EmulatedRegExp_instances, execCore_fn).call(this, str.slice(pos));
      if (match) {
        adjustMatchDetailsForOffset(match, pos, str, this.hasIndices);
        this.lastIndex += pos;
      }
      return match;
    }
    return __privateMethod(this, __EmulatedRegExp_instances, execCore_fn).call(this, str);
  }
}, _captureMap = new WeakMap(), _compiled = new WeakMap(), _pattern = new WeakMap(), _nameMap = new WeakMap(), _strategy = new WeakMap(), __EmulatedRegExp_instances = new WeakSet(), /**
Adds support for hidden and transfer captures.
@param {string} str
@returns
*/
execCore_fn = function(str) {
  __privateGet(this, _compiled).lastIndex = this.lastIndex;
  const match = __superGet(_c.prototype, this, "exec").call(__privateGet(this, _compiled), str);
  this.lastIndex = __privateGet(this, _compiled).lastIndex;
  if (!match || !__privateGet(this, _captureMap).size) {
    return match;
  }
  const matchCopy = [...match];
  match.length = 1;
  let indicesCopy;
  if (this.hasIndices) {
    indicesCopy = [...match.indices];
    match.indices.length = 1;
  }
  const mappedNums = [0];
  for (let i2 = 1; i2 < matchCopy.length; i2++) {
    const { hidden, transferTo } = __privateGet(this, _captureMap).get(i2) ?? {};
    if (hidden) {
      mappedNums.push(null);
    } else {
      mappedNums.push(match.length);
      match.push(matchCopy[i2]);
      if (this.hasIndices) {
        match.indices.push(indicesCopy[i2]);
      }
    }
    if (transferTo && matchCopy[i2] !== void 0) {
      const to = mappedNums[transferTo];
      if (!to) {
        throw new Error(`Invalid capture transfer to "${to}"`);
      }
      match[to] = matchCopy[i2];
      if (this.hasIndices) {
        match.indices[to] = indicesCopy[i2];
      }
      if (match.groups) {
        if (!__privateGet(this, _nameMap)) {
          __privateSet(this, _nameMap, createNameMap(this.source));
        }
        const name = __privateGet(this, _nameMap).get(transferTo);
        if (name) {
          match.groups[name] = matchCopy[i2];
          if (this.hasIndices) {
            match.indices.groups[name] = indicesCopy[i2];
          }
        }
      }
    }
  }
  return match;
}, _c);
function adjustMatchDetailsForOffset(match, offset, input, hasIndices) {
  match.index += offset;
  match.input = input;
  if (hasIndices) {
    const indices = match.indices;
    for (let i2 = 0; i2 < indices.length; i2++) {
      const arr = indices[i2];
      if (arr) {
        indices[i2] = [arr[0] + offset, arr[1] + offset];
      }
    }
    const groupIndices = indices.groups;
    if (groupIndices) {
      Object.keys(groupIndices).forEach((key2) => {
        const arr = groupIndices[key2];
        if (arr) {
          groupIndices[key2] = [arr[0] + offset, arr[1] + offset];
        }
      });
    }
  }
}
function createCaptureMap(hiddenCaptures, transfers) {
  const captureMap = /* @__PURE__ */ new Map();
  for (const num of hiddenCaptures) {
    captureMap.set(num, {
      hidden: true
    });
  }
  for (const [to, from] of transfers) {
    for (const num of from) {
      getOrInsert(captureMap, num, {}).transferTo = to;
    }
  }
  return captureMap;
}
function createNameMap(pattern) {
  const re2 = /(?<capture>\((?:\?<(?![=!])(?<name>[^>]+)>|(?!\?)))|\\?./gsu;
  const map = /* @__PURE__ */ new Map();
  let numCharClassesOpen = 0;
  let numCaptures = 0;
  let match;
  while (match = re2.exec(pattern)) {
    const { 0: m2, groups: { capture, name } } = match;
    if (m2 === "[") {
      numCharClassesOpen++;
    } else if (!numCharClassesOpen) {
      if (capture) {
        numCaptures++;
        if (name) {
          map.set(numCaptures, name);
        }
      }
    } else if (m2 === "]") {
      numCharClassesOpen--;
    }
  }
  return map;
}
function toRegExp(pattern, options) {
  const d2 = toRegExpDetails(pattern, options);
  if (d2.options) {
    return new EmulatedRegExp(d2.pattern, d2.flags, d2.options);
  }
  return new RegExp(d2.pattern, d2.flags);
}
function toRegExpDetails(pattern, options) {
  const opts = getOptions(options);
  const onigurumaAst = J(pattern, {
    flags: opts.flags,
    normalizeUnknownPropertyNames: true,
    rules: {
      captureGroup: opts.rules.captureGroup,
      singleline: opts.rules.singleline
    },
    skipBackrefValidation: opts.rules.allowOrphanBackrefs,
    unicodePropertyMap: JsUnicodePropertyMap
  });
  const regexPlusAst = transform(onigurumaAst, {
    accuracy: opts.accuracy,
    asciiWordBoundaries: opts.rules.asciiWordBoundaries,
    avoidSubclass: opts.avoidSubclass,
    bestEffortTarget: opts.target
  });
  const generated = generate(regexPlusAst, opts);
  const recursionResult = recursion(generated.pattern, {
    captureTransfers: generated._captureTransfers,
    hiddenCaptures: generated._hiddenCaptures,
    mode: "external"
  });
  const possessiveResult = possessive(recursionResult.pattern);
  const atomicResult = atomic(possessiveResult.pattern, {
    captureTransfers: recursionResult.captureTransfers,
    hiddenCaptures: recursionResult.hiddenCaptures
  });
  const details = {
    pattern: atomicResult.pattern,
    flags: `${opts.hasIndices ? "d" : ""}${opts.global ? "g" : ""}${generated.flags}${generated.options.disable.v ? "u" : "v"}`
  };
  if (opts.avoidSubclass) {
    if (opts.lazyCompileLength !== Infinity) {
      throw new Error("Lazy compilation requires subclass");
    }
  } else {
    const hiddenCaptures = atomicResult.hiddenCaptures.sort((a, b2) => a - b2);
    const transfers = Array.from(atomicResult.captureTransfers);
    const strategy = regexPlusAst._strategy;
    const lazyCompile = details.pattern.length >= opts.lazyCompileLength;
    if (hiddenCaptures.length || transfers.length || strategy || lazyCompile) {
      details.options = {
        ...hiddenCaptures.length && { hiddenCaptures },
        ...transfers.length && { transfers },
        ...strategy && { strategy },
        ...lazyCompile && { lazyCompile }
      };
    }
  }
  return details;
}
const MAX = 4294967295;
class JavaScriptScanner {
  constructor(patterns, options = {}) {
    __publicField(this, "regexps");
    this.patterns = patterns;
    this.options = options;
    const {
      forgiving = false,
      cache,
      regexConstructor
    } = options;
    if (!regexConstructor) {
      throw new Error("Option `regexConstructor` is not provided");
    }
    this.regexps = patterns.map((p2) => {
      if (typeof p2 !== "string") {
        return p2;
      }
      const cached = cache == null ? void 0 : cache.get(p2);
      if (cached) {
        if (cached instanceof RegExp) {
          return cached;
        }
        if (forgiving)
          return null;
        throw cached;
      }
      try {
        const regex = regexConstructor(p2);
        cache == null ? void 0 : cache.set(p2, regex);
        return regex;
      } catch (e) {
        cache == null ? void 0 : cache.set(p2, e);
        if (forgiving)
          return null;
        throw e;
      }
    });
  }
  findNextMatchSync(string, startPosition, _options) {
    const str = typeof string === "string" ? string : string.content;
    const pending = [];
    function toResult(index, match, offset = 0) {
      return {
        index,
        captureIndices: match.indices.map((indice) => {
          if (indice == null) {
            return {
              start: MAX,
              end: MAX,
              length: 0
            };
          }
          return {
            start: indice[0] + offset,
            end: indice[1] + offset,
            length: indice[1] - indice[0]
          };
        })
      };
    }
    for (let i2 = 0; i2 < this.regexps.length; i2++) {
      const regexp = this.regexps[i2];
      if (!regexp)
        continue;
      try {
        regexp.lastIndex = startPosition;
        const match = regexp.exec(str);
        if (!match)
          continue;
        if (match.index === startPosition) {
          return toResult(i2, match, 0);
        }
        pending.push([i2, match, 0]);
      } catch (e) {
        if (this.options.forgiving)
          continue;
        throw e;
      }
    }
    if (pending.length) {
      const minIndex = Math.min(...pending.map((m2) => m2[1].index));
      for (const [i2, match, offset] of pending) {
        if (match.index === minIndex) {
          return toResult(i2, match, offset);
        }
      }
    }
    return null;
  }
}
function defaultJavaScriptRegexConstructor(pattern, options) {
  return toRegExp(
    pattern,
    {
      global: true,
      hasIndices: true,
      // This has no benefit for the standard JS engine, but it avoids a perf penalty for
      // precompiled grammars when constructing extremely long patterns that aren't always used
      lazyCompileLength: 3e3,
      rules: {
        // Needed since TextMate grammars merge backrefs across patterns
        allowOrphanBackrefs: true,
        // Improves search performance for generated regexes
        asciiWordBoundaries: true,
        // Follow `vscode-oniguruma` which enables this Oniguruma option by default
        captureGroup: true,
        // Oniguruma uses depth limit `20`; lowered here to keep regexes shorter and maybe
        // sometimes faster, but can be increased if issues reported due to low limit
        recursionLimit: 5,
        // Oniguruma option for `^`->`\A`, `$`->`\Z`; improves search performance without any
        // change in meaning since TM grammars search line by line
        singleline: true
      },
      ...options
    }
  );
}
function createJavaScriptRegexEngine(options = {}) {
  const _options = Object.assign(
    {
      target: "auto",
      cache: /* @__PURE__ */ new Map()
    },
    options
  );
  _options.regexConstructor || (_options.regexConstructor = (pattern) => defaultJavaScriptRegexConstructor(pattern, { target: _options.target }));
  return {
    createScanner(patterns) {
      return new JavaScriptScanner(patterns, _options);
    },
    createString(s2) {
      return {
        content: s2
      };
    }
  };
}
export {
  ShikiError$2 as ShikiError,
  addClassToHast,
  applyColorReplacements,
  bundledLanguages,
  bundledLanguagesAlias,
  bundledLanguagesBase,
  bundledLanguagesInfo,
  bundledThemes,
  bundledThemesInfo,
  codeToHast,
  codeToHtml,
  codeToTokens,
  codeToTokensBase,
  codeToTokensWithThemes,
  createBundledHighlighter,
  createCssVariablesTheme,
  createHighlighter,
  createHighlighterCore,
  createHighlighterCoreSync,
  createJavaScriptRegexEngine,
  createOnigurumaEngine,
  createPositionConverter,
  createShikiInternal,
  createShikiInternalSync,
  createSingletonShorthands,
  createdBundledHighlighter,
  defaultJavaScriptRegexConstructor,
  enableDeprecationWarnings,
  flatTokenVariants,
  getLastGrammarState,
  getSingletonHighlighter,
  getSingletonHighlighterCore,
  getTokenStyleObject,
  guessEmbeddedLanguages,
  hastToHtml,
  isNoneTheme,
  isPlainLang,
  isSpecialLang,
  isSpecialTheme,
  loadWasm,
  makeSingletonHighlighter,
  makeSingletonHighlighterCore,
  normalizeGetter,
  normalizeTheme,
  resolveColorReplacements,
  splitLines,
  splitToken,
  splitTokens,
  stringifyTokenStyle,
  toArray,
  tokenizeAnsiWithTheme,
  tokenizeWithTheme,
  tokensToHast,
  transformerDecorations,
  warnDeprecated
};
