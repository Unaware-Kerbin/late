import { Component, type ErrorInfo, type ReactNode } from "react";

export class AppErrorBoundary extends Component<{ children: ReactNode }, { error: string | null }> {
  state = { error: null as string | null };

  static getDerivedStateFromError(err: Error) {
    return { error: err.message || String(err) };
  }

  componentDidCatch(err: Error, info: ErrorInfo) {
    console.error("late ui crash", err, info.componentStack);
  }

  render() {
    if (this.state.error) {
      return (
        <div style={{ padding: 24, color: "#d7e3ee", fontFamily: "sans-serif", maxWidth: 40 * 16 }}>
          <h2 style={{ marginTop: 0 }}>Late UI crashed</h2>
          <p style={{ color: "#7d8fa3" }}>{this.state.error}</p>
          <button
            type="button"
            onClick={() => {
              this.setState({ error: null });
              window.location.reload();
            }}
          >
            Reload
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
