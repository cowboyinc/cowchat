import React, { useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { Renderer } from '@openuidev/react-lang';
import { library } from './library.jsx';
import { preflightView } from './preflight.mjs';
import '@openuidev/react-ui/styles/index.css';
import './style.css';

class Boundary extends React.Component {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  render() { return this.state.failed ? <p role="alert">This view could not be rendered.</p> : this.props.children; }
}
function Artifact() {
  const [snapshot, setSnapshot] = useState(null);
  const [status, setStatus] = useState('Connecting');
  const [invalid, setInvalid] = useState(null);
  useEffect(() => {
    const path = location.pathname.replace(/\/$/, '');
    let stopped = false;
    let active = false;
    let pending = false;
    let retry;
    let streamRetry;
    let reconnectDelay = 500;
    let stream = null;
    const controller = new AbortController();
    const revoke = () => {
      stopped = true;
      clearTimeout(retry);
      clearTimeout(streamRetry);
      stream?.close();
      controller.abort();
      setSnapshot(null);
      setStatus('This artifact is unavailable.');
      document.title = 'Cowchat artifact';
    };
    const refresh = async () => {
      if (stopped) return;
      if (active) { pending = true; return; }
      active = true;
      clearTimeout(retry);
      do {
        pending = false;
        try {
          const response = await fetch(`${path}/snapshot`, { cache: 'no-store', credentials: 'omit', signal: controller.signal });
          if ([404, 410].includes(response.status)) {
            revoke();
            return;
          }
          if (!response.ok) throw new Error('Snapshot unavailable');
          const next = await response.json();
          if (stopped) return;
          if (typeof next.view !== 'string' || !next.data || Array.isArray(next.data) || typeof next.data !== 'object' || !Number.isSafeInteger(next.revision)) throw new Error('Invalid snapshot');
          try { next.view = preflightView(next.view,next.data,library); }
          catch { next.view = null; }
          setSnapshot(previous => !previous || next.revision >= previous.revision ? next : previous);
          setStatus(stream?.readyState === EventSource.OPEN ? 'Live' : 'Reconnecting');
        } catch (error) {
          if (!stopped) { setStatus('Reconnecting'); retry = setTimeout(refresh, 3000); }
        }
      } while (pending && !stopped);
      active = false;
    };
    // Native reconnect handles clean disconnects. Non-200/wrong-MIME responses
    // permanently close EventSource, so replace those instances with capped backoff.
    const connect = () => {
      if (stopped) return;
      const current = new EventSource(`${path}/events`);
      stream = current;
      const belongs = () => !stopped && stream === current;
      current.addEventListener('open', () => {
        if (!belongs()) return;
        reconnectDelay = 500;
        refresh();
      });
      current.addEventListener('revision', () => { if (belongs()) refresh(); });
      current.addEventListener('revoked', () => { if (belongs()) revoke(); });
      current.addEventListener('error', () => {
        if (!belongs()) return;
        setStatus('Reconnecting');
        if (current.readyState === EventSource.CLOSED) {
          stream = null;
          current.close();
          streamRetry = setTimeout(connect, reconnectDelay);
          reconnectDelay = Math.min(10000, reconnectDelay * 2);
        }
        refresh();
      });
    };
    // Subscribe before reading; every successful connect refetches the snapshot.
    connect();
    refresh();
    return () => {
      stopped = true;
      clearTimeout(retry);
      clearTimeout(streamRetry);
      stream?.close();
      controller.abort();
    };
  }, []);
  useEffect(() => { document.title = snapshot ? `${snapshot.title} · Cowchat` : 'Cowchat artifact'; }, [snapshot]);
  const state = Object.fromEntries(Object.entries(snapshot?.data ?? {}).map(([key, value]) => [`$${key}`, value]));
  return <div className="artifact-shell">
    <header className="artifact-header"><a className="artifact-wordmark" href="/" aria-label="Cowchat home">cowchat<span> / live</span></a>
      <span className={`artifact-status ${status === 'Live' ? 'is-live' : ''}`} role="status">{status}</span></header>
    {snapshot ? <main><div className="artifact-title"><h1>{snapshot.title}</h1><p>Last updated <time dateTime={snapshot.updated_at}>{new Date(snapshot.updated_at).toLocaleString()}</time></p></div>
      {!snapshot.view || invalid === snapshot.revision ? <p role="alert" data-revision={snapshot.revision}>This view could not be rendered.</p> : <Boundary key={snapshot.revision}><div className="artifact-content" data-revision={snapshot.revision}><Renderer key={snapshot.revision}
        response={snapshot.view} initialState={state} library={library} isStreaming={false} publishObservability={false}
        onError={errors => { if (errors.length) setInvalid(snapshot.revision); }}/></div></Boundary>}
      </main> : <main><h1>{status === 'Connecting' ? 'Loading artifact…' : status}</h1></main>}
  </div>;
}
createRoot(document.querySelector('#artifact')).render(<Artifact/>);
