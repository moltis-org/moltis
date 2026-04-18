/**
 * Copyright (c) 2014-2024 The xterm.js authors. All rights reserved.
 * @license MIT
 *
 * Copyright (c) 2012-2013, Christopher Jeffrey (MIT License)
 * @license MIT
 *
 * Originally forked from (with the author's permission):
 *   Fabrice Bellard's javascript vt100 for jslinux:
 *   http://bellard.org/jslinux/
 *   Copyright (c) 2011 Fabrice Bellard
 */var c=2,d=1,_=class{activate(e){this._terminal=e}dispose(){}fit(){let e=this.proposeDimensions();if(!e||!this._terminal||isNaN(e.cols)||isNaN(e.rows))return;let i=this._terminal._core;(this._terminal.rows!==e.rows||this._terminal.cols!==e.cols)&&(i._renderService.clear(),this._terminal.resize(e.cols,e.rows))}proposeDimensions(){var l;if(!this._terminal||!this._terminal.element||!this._terminal.element.parentElement)return;let e=this._terminal._core._renderService.dimensions;if(e.css.cell.width===0||e.css.cell.height===0)return;let i=this._terminal.options.scrollback===0?0:((l=this._terminal.options.overviewRuler)==null?void 0:l.width)||14,s=window.getComputedStyle(this._terminal.element.parentElement),o=parseInt(s.getPropertyValue("height")),a=Math.max(0,parseInt(s.getPropertyValue("width"))),t=window.getComputedStyle(this._terminal.element),r={top:parseInt(t.getPropertyValue("padding-top")),bottom:parseInt(t.getPropertyValue("padding-bottom")),right:parseInt(t.getPropertyValue("padding-right")),left:parseInt(t.getPropertyValue("padding-left"))},n=r.top+r.bottom,h=r.right+r.left,m=o-n,p=a-h-i;return{cols:Math.max(c,Math.floor(p/e.css.cell.width)),rows:Math.max(d,Math.floor(m/e.css.cell.height))}}};export{_ as FitAddon};
