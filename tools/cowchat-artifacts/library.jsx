import React from 'react';
import { createLibrary, defineComponent } from '@openuidev/react-lang';
import { openuiLibrary } from '@openuidev/react-ui/genui-lib';
import { Chip } from '@cowboyinc/gallop-components';
import Markdown from 'react-markdown';
import { z } from 'zod/v4';

const text = z.string().max(16000);
const scalar = z.union([text, z.number().finite(), z.boolean(), z.null()]);
const children = z.array(z.any()).max(200);
export function safeHref(value) {
  try {
    const url = new URL(value);
    return ['https:', 'http:'].includes(url.protocol) ? url.href : undefined;
  } catch { return undefined; }
}
const show = value => value == null ? '—' : String(value);
const define = (name, props, component) => defineComponent({ name, props: z.object(props), component });
const Stack = define('Stack', { children, direction: z.enum(['column', 'row']).optional() }, ({ props, renderNode }) =>
  <div className={`artifact-stack ${props.direction === 'row' ? 'artifact-row' : ''}`}>{renderNode(props.children)}</div>);
const Card = define('Card', { children }, ({ props, renderNode }) => <section className="artifact-card">{renderNode(props.children)}</section>);
const Heading = define('Heading', { text }, ({ props }) => <h2>{props.text}</h2>);
const Text = define('Text', { text }, ({ props }) => <p className="artifact-text">{props.text}</p>);
const MarkdownBlock = define('Markdown', { text }, ({ props }) => <div className="artifact-markdown"><Markdown skipHtml
  disallowedElements={['img', 'input', 'form', 'iframe', 'script', 'style']}
  components={{ a: ({ href, children }) => safeHref(href) ? <a href={safeHref(href)} target="_blank" rel="noopener noreferrer">{children}</a> : <span>{children}</span> }}>{props.text}</Markdown></div>);
const Badge = define('Badge', { text }, ({ props }) => <Chip disabled className="artifact-badge">{props.text}</Chip>);
const Link = define('Link', { text, href: text }, ({ props }) => safeHref(props.href)
  ? <a href={safeHref(props.href)} target="_blank" rel="noopener noreferrer">{props.text} <span aria-hidden="true">↗</span></a>
  : <span>{props.text}</span>);
const Metric = define('Metric', { label: text, value: scalar.nullish(), detail: text.nullish() }, ({ props }) =>
  <div className="artifact-metric"><span className="metric-label">{props.label}</span><strong>{show(props.value)}</strong>{props.detail && <span className="metric-detail">{props.detail}</span>}</div>);
const cell = z.union([scalar, z.object({ text, href: text.optional(), badge: z.boolean().optional() })]);
const row = z.record(z.string(), cell);
const Table = define('Table', {
  columns: z.array(z.object({ key: z.string().min(1).max(100), label: text })).min(1).max(20),
  rows: z.union([z.array(row).max(1000), z.record(z.string(), row).refine(rows => Object.keys(rows).length <= 1000)]).nullish(),
  caption: text.optional(),
}, ({ props }) => {
  const rows = Array.isArray(props.rows) ? props.rows.map((row, index) => [String(index), row])
    : Object.entries(props.rows ?? {}).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0);
  return <div className="artifact-table-wrap"><table>
  {props.caption && <caption>{props.caption}</caption>}
  <thead><tr>{props.columns.map(column => <th key={column.key} scope="col">{column.label}</th>)}</tr></thead>
  <tbody>{rows.map(([key, row]) => <tr key={key}>{props.columns.map(column => {
    const value = row[column.key];
    const content = value && typeof value === 'object' ? value : null;
    return <td key={column.key}>{content ? content.badge ? <Chip disabled className="artifact-badge">{content.text}</Chip>
      : safeHref(content.href) ? <a href={safeHref(content.href)} target="_blank" rel="noopener noreferrer">{content.text} ↗</a> : content.text : show(value)}</td>;
  })}</tr>)}</tbody>
  </table>{!rows.length && <p className="artifact-empty">No data</p>}</div>;
});

// These definitions are shared with the existing offline Cowchat chart renderer.
export const chartNames = ['BarChart', 'LineChart', 'AreaChart', 'RadarChart', 'HorizontalBarChart', 'Series', 'PieChart', 'RadialChart', 'SingleStackedBarChart', 'Slice', 'ScatterChart', 'ScatterSeries', 'Point'];
export const library = createLibrary({ components: [Stack, Card, Heading, Text, MarkdownBlock, Badge, Link, Metric, Table,
  ...chartNames.map(name => openuiLibrary.components[name])], root: 'Stack' });
