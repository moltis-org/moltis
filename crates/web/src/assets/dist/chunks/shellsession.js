import shellscript from "./shellscript.js";
const lang = Object.freeze(JSON.parse('{"displayName":"Shell Session","fileTypes":["sh-session"],"name":"shellsession","patterns":[{"captures":{"1":{"name":"entity.other.prompt-prefix.shell-session"},"2":{"name":"punctuation.separator.prompt.shell-session"},"3":{"name":"source.shell","patterns":[{"include":"source.shell"}]}},"match":"^(?:((?:\\\\(\\\\S+\\\\)\\\\s*)?(?:sh\\\\S*?|\\\\w+\\\\S+[:@]\\\\S+(?:\\\\s+\\\\S+)?|\\\\[\\\\S+?[:@]\\\\N+?].*?))\\\\s*)?([#$%>❯➜\\\\p{Greek}])\\\\s+(.*)$"},{"match":"^.+$","name":"meta.output.shell-session"}],"scopeName":"text.shell-session","embeddedLangs":["shellscript"],"aliases":["console"]}'));
const shellsession = [
  ...shellscript,
  lang
];
export {
  shellsession as default
};
