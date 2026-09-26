import { render } from 'preact';
import './styles.css';

document.documentElement.dataset.theme = 'dark';
render(<div>加载中…</div>, document.getElementById('root')!);
