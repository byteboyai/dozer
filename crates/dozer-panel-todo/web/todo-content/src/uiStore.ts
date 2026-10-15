import type { StatusFilter, ViewPayload } from './types.ts';

/** 每个项目各自保留的纯前端视图状态(不回传 Rust)。 */
export interface ProjectUi {
  /** 已生效的搜索词(回车 / 点按钮 / 失焦提交后) */
  search: string;
  /** 搜索框草稿 */
  searchDraft: string;
  status: StatusFilter;
  addDraft: string;
  /** 新增框当前高度(px);null = 用 payload.add_height_px */
  addHeight: number | null;
  /** 上一次见到的 category_key;null = 还没收到过推送 */
  categoryKey: string | null;
  selectedId: number | null;
  /** 上一次从 Rust 收到的 selected_id,用来判断"是否变化" */
  lastRustSelected: number | null;
  scrollNonce: number;
  scrollTop: number;
}

function fresh(): ProjectUi {
  return {
    search: '',
    searchDraft: '',
    status: 'all',
    addDraft: '',
    addHeight: null,
    categoryKey: null,
    selectedId: null,
    lastRustSelected: null,
    scrollNonce: 0,
    scrollTop: 0,
  };
}

export class UiStore {
  private m = new Map<number, ProjectUi>();

  get(projectId: number): ProjectUi {
    let ui = this.m.get(projectId);
    if (!ui) {
      ui = fresh();
      this.m.set(projectId, ui);
    }
    return ui;
  }

  /** 应用一条推送带来的、需要前端状态响应的变化。返回是否应滚回顶部。 */
  onPayload(p: ViewPayload): { scrollToTop: boolean } {
    const ui = this.get(p.project_id);
    // 分类切换:清搜索(草稿与生效词),沿用原生 `clear_search` 语义;不动新增草稿。
    if (ui.categoryKey !== null && ui.categoryKey !== p.category_key) {
      ui.search = '';
      ui.searchDraft = '';
    }
    ui.categoryKey = p.category_key;
    // Rust 的 selected_id 只在它"变化"时覆盖本地选择(新增后 2 秒高亮)。
    if (p.selected_id !== ui.lastRustSelected) {
      ui.selectedId = p.selected_id;
      ui.lastRustSelected = p.selected_id;
    }
    let scrollToTop = false;
    if (p.scroll_nonce !== ui.scrollNonce) {
      ui.scrollNonce = p.scroll_nonce;
      scrollToTop = true;
    }
    return { scrollToTop };
  }
}
