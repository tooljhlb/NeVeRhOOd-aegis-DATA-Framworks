import * as monaco from 'monaco-editor/editor/editor.api.js';
import 'monaco-editor/basic-languages/monaco.contribution.js';
self.MonacoEnvironment = { getWorker: () => new Worker('/editor-assets/editor.worker.js', { type: 'module' }) };
const languageFor = path => {
  const name = path.split('/').pop();
  const extension = name.includes('.') ? '.' + name.split('.').pop().toLowerCase() : '';
  const languages = monaco.languages.getLanguages();
  return languages.find(language => language.filenames?.includes(name))?.id || languages.find(language => language.extensions?.includes(extension))?.id || 'plaintext';
};
export function mount(element, onChange) {
  const editor = monaco.editor.create(element, { theme: 'vs-dark', automaticLayout: true, minimap: { enabled: false }, ariaLabel: 'AEGIS Monaco source editor', model: null });
  const models = new Map(); let updating = false;
  const subscription = editor.onDidChangeModelContent(() => { if (!updating) onChange(editor.getValue()); });
  return {
    show(path, text) { updating = true; let model = models.get(path); if (!model) { model = monaco.editor.createModel(text, languageFor(path), monaco.Uri.from({ scheme: 'aegis', path: '/' + path })); models.set(path, model); } else if (model.getValue() !== text) model.setValue(text); editor.setModel(model); updating = false; },
    markers(path, diagnostics) { const model = models.get(path); if (!model) return; const severity = { 1: monaco.MarkerSeverity.Error, 2: monaco.MarkerSeverity.Warning, 3: monaco.MarkerSeverity.Info, 4: monaco.MarkerSeverity.Hint }; monaco.editor.setModelMarkers(model, 'aegis-lsp', diagnostics.filter(d => d.range && typeof d.message === 'string').map(d => ({ message: d.message, severity: severity[d.severity] || monaco.MarkerSeverity.Info, source: d.source || 'LSP', code: d.code === undefined ? undefined : String(d.code), startLineNumber: d.range.start.line + 1, startColumn: d.range.start.character + 1, endLineNumber: d.range.end.line + 1, endColumn: d.range.end.character + 1 }))); },
    focus() { editor.focus(); },
    dispose() { subscription.dispose(); editor.dispose(); for (const model of models.values()) model.dispose(); models.clear(); }
  };
}
