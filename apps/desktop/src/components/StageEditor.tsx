import { useEffect, useRef, useState } from "react";
import "monaco-editor/min/vs/style.css";
import { useApp } from "../store";
import { xtermTheme } from "../appearance";
import {
  stageLanguageLabel,
  stageMonacoLanguage,
  stageSecretHits,
  stageSuggestions,
  type StageFormat,
} from "../lib/stageEditorLang";

type MonacoNs = typeof import("monaco-editor/esm/vs/editor/editor.api");
type CodeEditor = import("monaco-editor").editor.IStandaloneCodeEditor;
type CodeModel = import("monaco-editor").editor.ITextModel;

let providersReady = false;
const formatByModel = new Map<string, StageFormat>();

function cssVar(name: string, fallback: string): string {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return v || fallback;
}

function defineLateTheme(monaco: MonacoNs, themeId: string) {
  const term = xtermTheme(themeId as Parameters<typeof xtermTheme>[0]);
  const bg = cssVar("--panel", term.background);
  const fg = cssVar("--text", term.foreground);
  const accent = cssVar("--accent", term.cursor);
  const line = cssVar("--line", "#223042");
  const muted = cssVar("--muted", "#7d8fa3");
  const danger = cssVar("--danger", "#ff5d6a");
  const info = cssVar("--info", "#6cb6ff");
  const amber = cssVar("--amber", "#f5c542");
  const paper = themeId === "paper";
  monaco.editor.defineTheme("late-stage", {
    base: paper ? "vs" : "vs-dark",
    inherit: true,
    colors: {
      "editor.background": bg,
      "editor.foreground": fg,
      "editor.lineHighlightBackground": cssVar("--panel-2", "#151e29"),
      "editor.selectionBackground": term.selectionBackground,
      "editorCursor.foreground": accent,
      "editorLineNumber.foreground": muted,
      "editorLineNumber.activeForeground": fg,
      "editorIndentGuide.background": line,
      "editorWidget.background": cssVar("--bg-raise", bg),
      "editorWidget.border": line,
      "editorSuggestWidget.background": cssVar("--bg-raise", bg),
      "editorSuggestWidget.border": line,
      "editorSuggestWidget.foreground": fg,
      "editorSuggestWidget.selectedBackground": cssVar("--accent-dim", "#1a4f40"),
      "editorError.foreground": danger,
      "editorWarning.foreground": amber,
      "focusBorder": accent,
      "scrollbarSlider.background": line,
    },
    rules: [
      { token: "comment", foreground: muted.replace("#", "") },
      { token: "string", foreground: accent.replace("#", "") },
      { token: "keyword", foreground: info.replace("#", "") },
      { token: "number", foreground: amber.replace("#", "") },
    ],
  });
}

function ensureProviders(monaco: MonacoNs) {
  if (providersReady) return;
  providersReady = true;
  const langs = ["plaintext", "yaml", "python", "ruby"];
  for (const language of langs) {
    monaco.languages.registerCompletionItemProvider(language, {
      triggerCharacters: [".", ":", "-", "_"],
      provideCompletionItems(model, position) {
        const format = formatByModel.get(model.uri.toString());
        if (!format) return { suggestions: [] };
        if (stageMonacoLanguage(format) !== language) return { suggestions: [] };
        const word = model.getWordUntilPosition(position);
        const range = {
          startLineNumber: position.lineNumber,
          endLineNumber: position.lineNumber,
          startColumn: word.startColumn,
          endColumn: word.endColumn,
        };
        return {
          suggestions: stageSuggestions(format).map((s, i) => ({
            label: s.label,
            kind: monaco.languages.CompletionItemKind.Snippet,
            insertText: s.insertText,
            insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
            detail: s.detail,
            documentation: s.documentation,
            range,
            sortText: String(i).padStart(3, "0"),
          })),
        };
      },
    });
  }
}

function paintSecrets(monaco: MonacoNs, model: CodeModel) {
  const hits = stageSecretHits(model.getValue());
  monaco.editor.setModelMarkers(
    model,
    "late-stage",
    hits.map((h) => ({
      startLineNumber: h.line,
      startColumn: 1,
      endLineNumber: h.line,
      endColumn: 1000,
      message: h.text,
      severity: monaco.MarkerSeverity.Error,
    })),
  );
}

export function StageEditor({
  format,
  value,
  onChange,
  placeholder,
}: {
  format: StageFormat;
  value: string;
  onChange: (next: string) => void;
  placeholder: string;
}) {
  const hostRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<CodeEditor | null>(null);
  const monacoRef = useRef<MonacoNs | null>(null);
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  const valueRef = useRef(value);
  valueRef.current = value;
  const [failed, setFailed] = useState(false);
  const theme = useApp((s) => s.theme);
  const termFontSize = useApp((s) => s.termFontSize);
  const termFont = useApp((s) => s.termFont);
  const termFontCustom = useApp((s) => s.termFontCustom);

  useEffect(() => {
    let dead = false;
    let sub: { dispose: () => void } | undefined;
    void (async () => {
      try {
        await import("../lib/monacoEnv");
        await import("monaco-editor/esm/vs/editor/editor.all");
        const monaco = await import("monaco-editor/esm/vs/editor/editor.api");
        await Promise.all([
          import("monaco-editor/esm/vs/basic-languages/python/python.contribution"),
          import("monaco-editor/esm/vs/basic-languages/yaml/yaml.contribution"),
          import("monaco-editor/esm/vs/basic-languages/ruby/ruby.contribution"),
        ]);
        if (dead || !hostRef.current || editorRef.current) return;
        monacoRef.current = monaco;
        ensureProviders(monaco);
        defineLateTheme(monaco, theme);
        const lang = stageMonacoLanguage(format);
        const model = monaco.editor.createModel(valueRef.current, lang, monaco.Uri.parse(`inmemory://late/stage/${crypto.randomUUID()}`));
        formatByModel.set(model.uri.toString(), format);
        const editor = monaco.editor.create(hostRef.current, {
          model,
          theme: "late-stage",
          automaticLayout: true,
          minimap: { enabled: false },
          fontSize: termFontSize,
          fontFamily: getComputedStyle(document.documentElement).getPropertyValue("--term-font-family").trim() || "ui-monospace, monospace",
          lineNumbers: "on",
          wordWrap: "on",
          scrollBeyondLastLine: false,
          renderLineHighlight: "line",
          tabSize: lang === "python" ? 4 : 2,
          insertSpaces: true,
          quickSuggestions: { other: true, comments: false, strings: false },
          suggestOnTriggerCharacters: true,
          tabCompletion: "on",
          mouseWheelZoom: false,
          contextmenu: true,
          folding: true,
          bracketPairColorization: { enabled: true },
          padding: { top: 8, bottom: 8 },
          ariaLabel: "Staging draft",
          links: false,
          placeholder,
        });
        editorRef.current = editor;
        if (editor.getValue() !== valueRef.current) editor.setValue(valueRef.current);
        paintSecrets(monaco, model);
        sub = editor.onDidChangeModelContent(() => {
          const text = editor.getValue();
          onChangeRef.current(text);
          paintSecrets(monaco, model);
        });
      } catch (err) {
        console.error("late stage editor", err);
        if (!dead) setFailed(true);
      }
    })();
    return () => {
      dead = true;
      sub?.dispose();
      const ed = editorRef.current;
      const model = ed?.getModel();
      if (model) formatByModel.delete(model.uri.toString());
      model?.dispose();
      ed?.dispose();
      editorRef.current = null;
    };
    // Create once; language/theme/font sync in the effects below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const ed = editorRef.current;
    const monaco = monacoRef.current;
    const model = ed?.getModel();
    if (!ed || !monaco || !model) return;
    if (ed.getValue() !== value) ed.setValue(value);
  }, [value]);

  useEffect(() => {
    const monaco = monacoRef.current;
    const model = editorRef.current?.getModel();
    if (!monaco || !model) return;
    formatByModel.set(model.uri.toString(), format);
    monaco.editor.setModelLanguage(model, stageMonacoLanguage(format));
    editorRef.current?.updateOptions({ tabSize: stageMonacoLanguage(format) === "python" ? 4 : 2 });
  }, [format]);

  useEffect(() => {
    const monaco = monacoRef.current;
    if (!monaco) return;
    defineLateTheme(monaco, theme);
    monaco.editor.setTheme("late-stage");
  }, [theme]);

  useEffect(() => {
    editorRef.current?.updateOptions({
      fontSize: termFontSize,
      fontFamily:
        getComputedStyle(document.documentElement).getPropertyValue("--term-font-family").trim() ||
        "ui-monospace, monospace",
      placeholder,
    });
  }, [termFontSize, termFont, termFontCustom, placeholder]);

  if (failed) {
    return (
      <textarea
        className="stage-body"
        spellCheck={false}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        aria-label="Staging draft"
      />
    );
  }

  return (
    <div className="stage-editor-wrap">
      <div ref={hostRef} className="stage-editor" />
      <div className="stage-editor-chrome">
        <span>{stageLanguageLabel(format)}</span>
        <span>Ctrl+Space suggestions · Tab indent</span>
      </div>
    </div>
  );
}
