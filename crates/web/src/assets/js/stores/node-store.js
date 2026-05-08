const M = (window.__moltis_modules || {})["stores/node-store"] || {};
export default M;
export const nodes = M.nodes;
export const selectedNodeId = M.selectedNodeId;
export const nodeStore = M.nodeStore;
export const setAll = (...args) => M.setAll?.(...args);
export const select = (...args) => M.select?.(...args);
export const getById = (...args) => M.getById?.(...args);
export const selectedNode = M.selectedNode;
