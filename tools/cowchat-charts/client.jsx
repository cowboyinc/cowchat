import React from 'react';
import { createRoot } from 'react-dom/client';
import { flushSync } from 'react-dom';
import { Renderer, createLibrary } from '@openuidev/react-lang';
import { openuiLibrary } from '@openuidev/react-ui/genui-lib';
import '@openuidev/react-ui/styles/index.css';
import './style.css';

// No URLs, images, forms, tools, custom code, or actions are available to specs.
const allowed = ['Stack', 'TextContent', 'Card', 'CardHeader', 'BarChart', 'LineChart',
  'AreaChart', 'RadarChart', 'HorizontalBarChart', 'Series', 'PieChart', 'RadialChart',
  'SingleStackedBarChart', 'Slice', 'ScatterChart', 'ScatterSeries', 'Point'];
const library = createLibrary({ components: allowed.map(name => openuiLibrary.components[name]), root: 'Stack' });
const root = createRoot(document.querySelector('#chart'));
class Boundary extends React.Component {
  state = { error: null };
  static getDerivedStateFromError(error) { return { error: String(error) }; }
  render() {
    if (this.state.error) { window.chartError = this.state.error; return null; }
    return this.props.children;
  }
}
window.renderChart = response => {
  window.chartError = null;
  flushSync(() => root.render(<Boundary><Renderer response={response} library={library} isStreaming={false} publishObservability={false} onError={errors => { if (errors.length) window.chartError = errors.map(e => e.message).join("; "); }} /></Boundary>));
};
