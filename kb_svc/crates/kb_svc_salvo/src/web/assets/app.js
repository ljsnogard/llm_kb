/*
 * kb_svc_salvo 前端脚本
 *
 * 目标：用最少的代码把「提问 → 增量显示 → 结束」这条链路跑通，并提供一个
 * 配置 LLM 服务与 API key 的面板。刻意不引入任何框架或构建步骤：
 * 全部通过 DOM API 构造节点（不用 innerHTML 拼接模型输出），因此不存在
 * 把模型文本当作 HTML 解析的风险。
 *
 * 与后端的契约见 kb_svc_salvo::wire：
 *   → {type:"ask"|"cancel"|"use_service", ...}
 *   ← {type:"ready"|"started"|"delta"|"tool_call"|"usage"|"finished"|"error", ...}
 */
(() => {
  "use strict";

  // ── 工具 ──────────────────────────────────────────────────────────

  const $ = (id) => document.getElementById(id);

  /** 构造一个元素并可选地设置类名与文本。 */
  function el(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }

  /** 判断滚动容器是否已经贴近底部。 */
  function isNearBottom(node, slack = 48) {
    return node.scrollHeight - node.scrollTop - node.clientHeight <= slack;
  }

  // ── 状态 ──────────────────────────────────────────────────────────

  /** 一条聊天记录。 */
  class Turn {
    constructor(role, text) {
      this.role = role; // "user" | "assistant"
      this.text = text || "";
      this.reasoning = "";
      this.toolCalls = [];
      this.usage = null;
      this.notice = null;
      this.state = role === "user" ? "done" : "streaming";
    }
  }

  const state = {
    /** 与后端的 WebSocket。 */
    socket: null,
    /** 当前是否已连上后端。 */
    connected: false,
    /** 重连退避序号。 */
    retry: 0,
    /** 后端上报的插件在线状态。 */
    pluginOnline: false,
    /** 聊天记录。 */
    turns: [],
    /** 当前正在生成的那条助手记录。 */
    active: null,
    /** 正在生成中的 turn 标识。 */
    activeTurnId: null,
    /** 已配置的服务摘要。 */
    services: [],
    /** 当前生效的服务标识。 */
    activeService: null,
    /** 设置面板是否展开。 */
    panelOpen: false,
    /** 正在编辑的服务标识；为空表示「新增」。 */
    editing: null,
    /** 渲染调度标志。 */
    framePending: false,
  };

  // ── DOM 引用 ──────────────────────────────────────────────────────

  const dom = {
    chat: $("chat"),
    messages: $("messages"),
    empty: $("chat-empty"),
    input: $("composer-input"),
    send: $("send-btn"),
    cancel: $("cancel-btn"),
    hint: $("composer-hint"),
    badge: $("plugin-badge"),
    serviceSelect: $("service-select"),
    themeToggle: $("theme-toggle"),
    settingsToggle: $("settings-toggle"),
    panel: $("panel"),
    panelClose: $("panel-close"),
    panelStatus: $("panel-status"),
    scrim: $("scrim"),
    serviceList: $("service-list"),
    configPath: $("config-path"),
    form: $("service-form"),
    formTitle: $("form-title"),
    formReset: $("form-reset"),
    fieldId: $("field-id"),
    fieldProvider: $("field-provider"),
    fieldModel: $("field-model"),
    fieldBaseUrl: $("field-base-url"),
    fieldApiKey: $("field-api-key"),
  };

  // ── 渲染 ──────────────────────────────────────────────────────────

  /** 在下一帧渲染（把同一帧内的多个 delta 合并成一次 DOM 更新）。 */
  function scheduleRender() {
    if (state.framePending) return;
    state.framePending = true;
    requestAnimationFrame(() => {
      state.framePending = false;
      render();
    });
  }

  function render() {
    const stick = isNearBottom(dom.chat);

    dom.empty.hidden = state.turns.length > 0;
    dom.messages.replaceChildren();

    for (const [index, turn] of state.turns.entries()) {
      if (turn.role === "user") {
        dom.messages.append(renderUser(turn));
      } else {
        dom.messages.append(renderAssistant(index, turn));
      }
    }

    if (stick) {
      dom.chat.scrollTop = dom.chat.scrollHeight;
    }

    // 输入区的可用状态
    const busy = state.activeTurnId !== null;
    dom.send.disabled = !state.connected || busy;
    dom.cancel.hidden = !busy;
  }

  function renderUser(turn) {
    const wrapper = el("div", "message message--user");
    wrapper.append(el("div", "message__bubble", turn.text));
    return wrapper;
  }

  function renderAssistant(index, turn) {
    const wrapper = el("div", "message message--assistant");
    const body = el("div", "message__body");

    if (turn.reasoning) {
      const detail = el("details", "reasoning");
      // 生成中默认展开，结束后折叠，与 DSH 的 reasoning 行行为一致。
      if (turn.state === "streaming") {
        detail.open = true;
        detail.dataset.streaming = "true";
      }

      const summary = el("summary", "reasoning__summary");
      summary.append(el("span", "reasoning__title", "思考过程"));
      summary.append(el("span", "reasoning__dot"));
      summary.append(
        el("span", "reasoning__state", turn.state === "streaming" ? "生成中" : "已完成"),
      );
      detail.append(summary);
      detail.append(el("div", "reasoning__text", turn.reasoning));
      body.append(detail);
    }

    for (const call of turn.toolCalls) {
      const box = el("div", "tool-call");
      box.append(el("span", "tool-call__name", call.name));
      box.append(document.createTextNode(call.arguments ? ` ${call.arguments}` : ""));
      body.append(box);
    }

    if (turn.text) {
      body.append(renderAnswer(turn));
    }

    if (turn.notice) {
      body.append(el("div", `notice notice--${turn.notice.tone}`, turn.notice.text));
    }

    if (turn.usage) {
      body.append(el("div", "usage", turn.usage));
    }

    if (!turn.text && !turn.reasoning && !turn.notice && turn.state === "streaming") {
      body.append(el("div", "message__meta", "等待模型输出…"));
    }

    wrapper.append(body);
    return wrapper;
  }

  /**
   * 渲染助手正文：把 ``` 围栏代码块单独渲染成 <pre>，其余作为纯文本。
   *
   * 落盘时（state !== "streaming"）才做完整切分；流式过程中只保证末段可见，
   * 避免每来一个 delta 就重建整个代码块。
   */
  function renderAnswer(turn) {
    const container = el("div", `answer${turn.state === "streaming" ? " streaming" : ""}`);

    const segments = splitCodeFences(turn.text);

    for (const segment of segments) {
      if (segment.code) {
        container.append(el("pre", "code", segment.text));
      } else if (segment.text) {
        container.append(document.createTextNode(segment.text));
      }
    }

    return container;
  }

  /**
   * 把文本按 ``` 围栏切成 [{text, code}] 片段。
   *
   * 未闭合的围栏按普通文本处理（流式过程中很常见），保证不会吞掉内容。
   */
  function splitCodeFences(text) {
    const parts = text.split("```");
    /** 围栏数量为奇数说明有一段未闭合，最后一段按普通文本处理。 */
    const hasOpenFence = parts.length % 2 === 0;
    const segments = [];

    for (let i = 0; i < parts.length; i += 1) {
      const isCode = i % 2 === 1 && !(hasOpenFence && i === parts.length - 1);
      if (parts[i] === "") continue;
      segments.push({ text: parts[i], code: isCode });
    }

    return segments;
  }

  /** 更新插件状态与提示文案（不涉及消息列表）。 */
  function renderChrome() {
    dom.badge.textContent = state.pluginOnline ? "插件在线" : "插件离线";
    dom.badge.dataset.online = state.pluginOnline ? "true" : "false";

    const services = state.services;
    dom.serviceSelect.replaceChildren();

    if (services.length === 0) {
      const option = el("option", null, "未配置服务");
      option.value = "";
      dom.serviceSelect.append(option);
      dom.serviceSelect.disabled = true;
    } else {
      dom.serviceSelect.disabled = false;
      for (const service of services) {
        const option = el("option", null, service.id);
        option.value = service.id;
        option.selected = service.id === state.activeService;
        dom.serviceSelect.append(option);
      }
    }
  }

  /** 设置底部提示。 */
  function setHint(text, tone) {
    dom.hint.textContent = text || "";
    if (tone) {
      dom.hint.dataset.tone = tone;
    } else {
      delete dom.hint.dataset.tone;
    }
  }

  /** 设置面板里的状态行。 */
  function setPanelStatus(text, tone) {
    dom.panelStatus.textContent = text || "";
    if (tone) {
      dom.panelStatus.dataset.tone = tone;
    } else {
      delete dom.panelStatus.dataset.tone;
    }
  }

  // ── 与后端的连接 ──────────────────────────────────────────────────

  function connect() {
    const scheme = location.protocol === "https:" ? "wss" : "ws";
    const socket = new WebSocket(`${scheme}://${location.host}/ws/chat`);
    state.socket = socket;

    socket.addEventListener("open", () => {
      state.connected = true;
      state.retry = 0;
      setHint("");
      render();
    });

    socket.addEventListener("message", (event) => {
      let message;
      try {
        message = JSON.parse(event.data);
      } catch (err) {
        console.warn("无法解析服务端消息", err, event.data);
        return;
      }
      handleServerMessage(message);
    });

    socket.addEventListener("close", () => {
      state.connected = false;
      state.activeTurnId = null;
      if (state.active && state.active.state === "streaming") {
        finishActive("已与服务端断开连接");
      }
      setHint("与服务端断开，正在重连…", "error");

      // 退避重连：0.5s、1s、2s…最多 8s。
      const delay = Math.min(8000, 500 * 2 ** state.retry);
      state.retry += 1;
      window.setTimeout(connect, delay);
      render();
    });

    socket.addEventListener("error", () => {
      // close 事件会紧随其后，重连逻辑放在那里。
    });
  }

  function sendToServer(payload) {
    if (!state.socket || state.socket.readyState !== WebSocket.OPEN) {
      setHint("与服务端未连接", "error");
      return false;
    }
    state.socket.send(JSON.stringify(payload));
    return true;
  }

  function handleServerMessage(message) {
    switch (message.type) {
      case "ready": {
        state.pluginOnline = Boolean(message.plugin_online);
        state.activeService = message.active_service ?? null;
        const ids = message.services ?? [];
        if (ids.length !== state.services.length ||
            ids.some((id, i) => state.services[i]?.id !== id)) {
          // 服务列表发生变化，重新拉取详情（含模型名等展示信息）。
          refreshSettings();
        }
        renderChrome();
        render();
        break;
      }

      case "started": {
        state.activeTurnId = message.turn_id;
        if (state.active) {
          state.active.state = "streaming";
          state.active.capabilities = message.capabilities ?? null;
        }
        scheduleRender();
        break;
      }

      case "delta": {
        if (!state.active) {
          // 例如刚刷新页面就收到上一轮的尾巴：开一条新的记录接住它。
          state.active = new Turn("assistant");
          state.turns.push(state.active);
          state.activeTurnId = message.turn_id;
        }
        // `logic` 与 abs_llm::v1 的 LogicOutput 一一对应；目前界面只区分
        // reasoning 与「其余（含 answer / 各类 search call）」两类展示。
        switch (message.logic) {
          case "reasoning":
            state.active.reasoning += message.text;
            break;
          case "answer":
            state.active.text += message.text;
            break;
          default:
            // function_call / dynamic_search_call / static_search_call 等：
            // 先按正文展示，后续再给它们各自的样式。
            state.active.text += message.text;
            break;
        }
        scheduleRender();
        break;
      }

      case "tool_call": {
        if (state.active) {
          state.active.toolCalls.push({ name: message.name, arguments: message.arguments });
          scheduleRender();
        }
        break;
      }

      case "usage": {
        if (state.active) {
          const usage = message.usage ?? {};
          const parts = [];
          if (usage.input_tokens != null) parts.push(`输入 ${usage.input_tokens}`);
          if (usage.output_tokens != null) parts.push(`输出 ${usage.output_tokens}`);
          if (usage.total_tokens != null) parts.push(`合计 ${usage.total_tokens}`);
          state.active.usage = parts.length ? `token：${parts.join(" / ")}` : "token：未知";
          scheduleRender();
        }
        break;
      }

      case "finished": {
        // 结束原因可能为 null（provider 不一定给），此时不显示额外提示。
        const reasonText = {
          completed: "",
          cancelled: "已取消",
          max_tokens: "达到输出上限",
          tool_call: "等待工具调用",
          other: "已结束",
        }[message.reason] ?? "";
        finishActive(reasonText);
        break;
      }

      case "error": {
        if (state.active) {
          state.active.notice = { tone: "error", text: message.message };
          finishActive();
        } else {
          state.turns.push(errorTurn(message.message));
          scheduleRender();
        }
        if (message.code === "plugin_offline") {
          state.pluginOnline = false;
          renderChrome();
        }
        break;
      }

      default:
        console.debug("忽略未知消息类型", message);
    }
  }

  /** 造一条只含错误的助手记录。 */
  function errorTurn(text) {
    const turn = new Turn("assistant");
    turn.state = "done";
    turn.notice = { tone: "error", text };
    return turn;
  }

  /** 结束当前生成中的记录。 */
  function finishActive(note) {
    if (state.active) {
      state.active.state = "done";
      if (note && !state.active.notice && !state.active.text) {
        state.active.notice = { tone: "info", text: note };
      }
      state.active = null;
    }
    state.activeTurnId = null;
    scheduleRender();
  }

  // ── 提问 / 取消 ───────────────────────────────────────────────────

  function submitQuestion() {
    const question = dom.input.value.trim();
    if (!question) return;

    if (state.services.length === 0) {
      setHint("请先在设置里添加一个 LLM 服务并填写 API key", "error");
      openPanel();
      return;
    }

    const service = state.activeService
      ? state.services.find((item) => item.id === state.activeService)
      : state.services[0];

    if (service && !service.has_api_key) {
      setHint(`服务 ${service.id} 还没有 API key，请在设置里补上`, "error");
      openPanel();
      return;
    }

    const turnId = crypto.randomUUID ? crypto.randomUUID().replaceAll("-", "") : String(Date.now());

    if (!sendToServer({ type: "ask", turn_id: turnId, question, service_id: service?.id ?? null })) {
      return;
    }

    state.turns.push(new Turn("user", question));
    state.active = new Turn("assistant");
    state.turns.push(state.active);
    state.activeTurnId = turnId;

    dom.input.value = "";
    autoGrow();
    setHint("");
    render();
  }

  function cancelActive() {
    if (!state.activeTurnId) return;
    sendToServer({ type: "cancel", turn_id: state.activeTurnId });
  }

  function autoGrow() {
    dom.input.style.height = "auto";
    dom.input.style.height = `${Math.min(160, dom.input.scrollHeight)}px`;
  }

  // ── 设置面板 ──────────────────────────────────────────────────────

  function openPanel() {
    state.panelOpen = true;
    dom.panel.hidden = false;
    dom.scrim.hidden = false;
    dom.settingsToggle.setAttribute("aria-expanded", "true");
    refreshSettings();
  }

  function closePanel() {
    state.panelOpen = false;
    dom.panel.hidden = true;
    dom.scrim.hidden = true;
    dom.settingsToggle.setAttribute("aria-expanded", "false");
  }

  async function refreshSettings() {
    try {
      const response = await fetch("/api/settings");
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const data = await response.json();

      state.services = data.services ?? [];
      state.activeService = data.active_service ?? null;
      dom.configPath.textContent = data.config_path ? `配置文件：${data.config_path}` : "";

      renderChrome();
      renderServiceList();
    } catch (err) {
      setPanelStatus(`读取设置失败：${err.message}`, "error");
    }
  }

  function renderServiceList() {
    dom.serviceList.replaceChildren();

    if (state.services.length === 0) {
      dom.serviceList.append(el("li", "service-item__detail", "还没有配置任何服务。"));
      return;
    }

    for (const service of state.services) {
      const item = el("li", "service-item");
      item.dataset.active = String(service.id === state.activeService);

      const main = el("div", "service-item__main");
      const name = el("div", "service-item__name", service.id);

      if (service.id === state.activeService) {
        name.append(el("span", "tag", "当前"));
      }
      if (!service.has_api_key) {
        name.append(el("span", "tag tag--warn", "缺少 API key"));
      }

      main.append(name);
      main.append(
        el(
          "div",
          "service-item__detail",
          `${service.provider} · ${service.model}${service.base_url ? ` · ${service.base_url}` : ""}`,
        ),
      );

      const actions = el("div", "service-item__actions");

      if (service.id !== state.activeService) {
        const use = el("button", "mini-btn", "使用");
        use.type = "button";
        use.addEventListener("click", () => {
          sendToServer({ type: "use_service", service_id: service.id });
          state.activeService = service.id;
          renderChrome();
          renderServiceList();
        });
        actions.append(use);
      }

      const edit = el("button", "mini-btn", "编辑");
      edit.type = "button";
      edit.addEventListener("click", () => startEdit(service));
      actions.append(edit);

      const remove = el("button", "mini-btn mini-btn--danger", "删除");
      remove.type = "button";
      remove.addEventListener("click", () => removeService(service.id));
      actions.append(remove);

      item.append(main);
      item.append(actions);
      dom.serviceList.append(item);
    }
  }

  function startEdit(service) {
    state.editing = service.id;
    dom.formTitle.textContent = `编辑服务：${service.id}`;
    dom.fieldId.value = service.id;
    dom.fieldId.readOnly = true;
    dom.fieldProvider.value = service.provider;
    dom.fieldModel.value = service.model;
    dom.fieldBaseUrl.value = service.base_url || "";
    dom.fieldApiKey.value = service.api_key || "";
    setPanelStatus("留空 API key 不会清除已保存的值；填入新值则覆盖。");
  }

  function resetForm() {
    state.editing = null;
    dom.formTitle.textContent = "新增服务";
    dom.form.reset();
    dom.fieldId.readOnly = false;
    setPanelStatus("");
  }

  async function submitServiceForm(event) {
    event.preventDefault();

    const payload = {
      id: dom.fieldId.value.trim(),
      provider: dom.fieldProvider.value.trim(),
      model: dom.fieldModel.value.trim(),
      base_url: dom.fieldBaseUrl.value.trim(),
      api_key: dom.fieldApiKey.value.trim(),
    };

    if (!payload.id || !payload.provider || !payload.model) {
      setPanelStatus("服务标识、Provider、模型三项都不能为空。", "error");
      return;
    }

    try {
      const response = await fetch("/api/settings/services", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(payload),
      });
      const data = await response.json().catch(() => ({}));

      if (!response.ok) {
        throw new Error(data.error || `HTTP ${response.status}`);
      }

      setPanelStatus(`已保存服务 ${payload.id}。`);
      // 保存后保持表单当前内容，方便继续微调；只有新增完成时清空。
      if (state.editing === null) {
        resetForm();
      }
      await refreshSettings();
    } catch (err) {
      setPanelStatus(`保存失败：${err.message}`, "error");
    }
  }

  async function removeService(id) {
    if (!window.confirm(`确定删除服务 ${id}？`)) return;

    try {
      const response = await fetch(`/api/settings/services/${encodeURIComponent(id)}`, {
        method: "DELETE",
      });
      if (!response.ok) {
        const data = await response.json().catch(() => ({}));
        throw new Error(data.error || `HTTP ${response.status}`);
      }
      setPanelStatus(`已删除服务 ${id}。`);
      if (state.editing === id) resetForm();
      await refreshSettings();
    } catch (err) {
      setPanelStatus(`删除失败：${err.message}`, "error");
    }
  }

  // ── 主题 ──────────────────────────────────────────────────────────

  function applyTheme(theme) {
    document.documentElement.dataset.theme = theme;
    try {
      localStorage.setItem("kb-theme", theme);
    } catch (err) {
      // 隐私模式下 localStorage 可能不可用，忽略即可。
    }
  }

  function toggleTheme() {
    const current = document.documentElement.dataset.theme === "light" ? "light" : "dark";
    applyTheme(current === "light" ? "dark" : "light");
  }

  function initTheme() {
    let theme = "dark";
    try {
      theme = localStorage.getItem("kb-theme") || "dark";
    } catch (err) {
      theme = "dark";
    }
    document.documentElement.dataset.theme = theme;
  }

  // ── 事件绑定 ──────────────────────────────────────────────────────

  function bindEvents() {
    dom.input.addEventListener("input", autoGrow);

    dom.input.addEventListener("keydown", (event) => {
      if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
        event.preventDefault();
        submitQuestion();
      }
    });

    dom.send.addEventListener("click", submitQuestion);
    dom.cancel.addEventListener("click", cancelActive);

    dom.serviceSelect.addEventListener("change", () => {
      const id = dom.serviceSelect.value;
      if (!id) return;
      sendToServer({ type: "use_service", service_id: id });
      state.activeService = id;
      renderServiceList();
    });

    dom.settingsToggle.addEventListener("click", () => {
      if (state.panelOpen) closePanel();
      else openPanel();
    });
    dom.panelClose.addEventListener("click", closePanel);
    dom.scrim.addEventListener("click", closePanel);

    dom.themeToggle.addEventListener("click", toggleTheme);

    dom.form.addEventListener("submit", submitServiceForm);
    dom.formReset.addEventListener("click", resetForm);

    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape" && state.panelOpen) closePanel();
    });
  }

  // ── 启动 ──────────────────────────────────────────────────────────

  function main() {
    initTheme();
    bindEvents();
    renderChrome();
    render();
    connect();
    refreshSettings();
  }

  main();
})();
