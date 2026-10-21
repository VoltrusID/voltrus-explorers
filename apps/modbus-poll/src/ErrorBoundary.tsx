import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  children: ReactNode;
}
interface State {
  error: Error | null;
}

// Catches render throws so the app shows a message + reload button instead of
// a permanent black screen (React unmounts the root on uncaught render errors).
export default class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // Surface to terminal too, since webview console isn't always visible.
    console.error("[render error]", error, info.componentStack);
  }

  render() {
    if (this.state.error) {
      return (
        <div style={{ padding: 24, fontSize: "var(--fs-sm)", color: "var(--danger)", background: "var(--bg)", height: "100vh", boxSizing: "border-box", overflow: "auto" }}>
          <h2 style={{ marginTop: 0, color: "var(--text)" }}>Render error</h2>
          <pre style={{ whiteSpace: "pre-wrap", wordBreak: "break-word" }}>
            {this.state.error.message}
            {"\n\n"}
            {this.state.error.stack}
          </pre>
          <button className="vt-btn" onClick={() => this.setState({ error: null })} style={{ marginTop: 12 }}>
            Try again
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
