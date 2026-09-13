// 设置面板。
//
// 侧边栏左下角的「设置」按钮打开它，用来配置 LLM 服务与 API key。
// 字段与行为对齐 `kb_svc_salvo` 网页端的设置抽屉（`#panel`）与
// `GET/POST /api/settings*` 接口：
//
// - 服务条目：「服务标识 / provider / 模型 / Base URL / API key」；
// - 每个条目显示「当前」「缺少 API key」标记，以及「使用 / 编辑 / 删除」；
// - 保存时若 API key 仍是遮蔽值，表示用户只想改别的字段。
//
// 当前阶段的取舍与网页端一致：**API key 明文保存在本地**
// （这里落在 `shared_preferences`，将来改为随工作区一起交给 `kb_core`）。

import 'package:flutter/material.dart';

import '../../models/llm_service.dart';
import '../../state/app_controller.dart';
import '../../theme/app_theme.dart';
import '../../theme/dsw_tokens.dart';
import '../common/dsw_controls.dart';

/// 打开设置面板。
Future<void> showSettingsDialog(
  BuildContext context,
  AppController controller,
) {
  return showDialog<void>(
    context: context,
    barrierColor: context.dsw.overlay,
    builder: (BuildContext context) => SettingsDialog(controller: controller),
  );
}

/// 设置面板本体。
class SettingsDialog extends StatefulWidget {
  /// 构建设置面板。
  const SettingsDialog({super.key, required this.controller});

  /// 应用状态。
  final AppController controller;

  @override
  State<SettingsDialog> createState() => _SettingsDialogState();
}

class _SettingsDialogState extends State<SettingsDialog> {
  final TextEditingController _id = TextEditingController();
  final TextEditingController _provider = TextEditingController();
  final TextEditingController _model = TextEditingController();
  final TextEditingController _baseUrl = TextEditingController();
  final TextEditingController _apiKey = TextEditingController();

  /// 正在编辑的服务标识；`null` 表示当前是「新增」。
  String? _editing;

  /// API key 是否明文显示。
  bool _revealKey = false;

  /// 底部状态行。
  String _status = '';
  bool _statusIsError = false;

  @override
  void dispose() {
    _id.dispose();
    _provider.dispose();
    _model.dispose();
    _baseUrl.dispose();
    _apiKey.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    final Size screen = MediaQuery.sizeOf(context);

    return Dialog(
      backgroundColor: c.bgLayer2,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(28)),
      child: ConstrainedBox(
        constraints: BoxConstraints(
          maxWidth: 760,
          maxHeight: (screen.height - 48).clamp(320.0, 800.0),
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: <Widget>[
            _Header(onClose: () => Navigator.of(context).pop()),
            Flexible(
              child: SingleChildScrollView(
                padding: const EdgeInsets.fromLTRB(24, 0, 24, 20),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: <Widget>[
                    const _PlaintextNotice(),
                    const SizedBox(height: 20),
                    const DswSectionLabel('LLM 服务', padding: EdgeInsets.zero),
                    const SizedBox(height: 6),
                    _ServiceList(
                      controller: widget.controller,
                      onUse: (LlmService service) {
                        widget.controller.setActiveService(service.id);
                        _setStatus('已切换到 ${service.id}');
                      },
                      onEdit: (LlmService service) => _loadIntoForm(service),
                      onDelete: (LlmService service) {
                        widget.controller.removeService(service.id);
                        if (_editing == service.id) {
                          _resetForm();
                        }
                        _setStatus('已删除 ${service.id}');
                      },
                    ),
                    const SizedBox(height: 24),
                    DswSectionLabel(
                      _editing == null ? '新增服务' : '编辑服务：$_editing',
                      padding: EdgeInsets.zero,
                    ),
                    const SizedBox(height: 8),
                    _buildForm(context),
                    const SizedBox(height: 20),
                    const DswSectionLabel('外观', padding: EdgeInsets.zero),
                    const SizedBox(height: 8),
                    _ThemeRow(controller: widget.controller),
                  ],
                ),
              ),
            ),
            _StatusBar(status: _status, isError: _statusIsError),
          ],
        ),
      ),
    );
  }

  Widget _buildForm(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: <Widget>[
            Expanded(
              child: DswLabeledField(
                label: '服务标识',
                controller: _id,
                hint: 'deepseek',
              ),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: DswLabeledField(
                label: 'Provider',
                controller: _provider,
                hint: 'deepseek',
              ),
            ),
          ],
        ),
        const SizedBox(height: 12),
        Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: <Widget>[
            Expanded(
              child: DswLabeledField(
                label: '模型',
                controller: _model,
                hint: 'deepseek-chat',
              ),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: DswLabeledField(
                label: 'Base URL（可留空）',
                controller: _baseUrl,
                hint: 'https://api.deepseek.com',
              ),
            ),
          ],
        ),
        const SizedBox(height: 12),
        DswLabeledField(
          label: 'API key',
          controller: _apiKey,
          hint: 'sk-...',
          obscure: !_revealKey,
          suffix: DswIconButton(
            tooltip: _revealKey ? '隐藏' : '显示',
            size: 28,
            iconSize: 16,
            icon: _revealKey ? Icons.visibility_off : Icons.visibility,
            onPressed: () => setState(() => _revealKey = !_revealKey),
          ),
        ),
        const SizedBox(height: 16),
        Row(
          children: <Widget>[
            DswPrimaryButton(label: '保存', onPressed: _save),
            const SizedBox(width: 8),
            DswGhostButton(label: '清空', onPressed: _resetForm),
          ],
        ),
      ],
    );
  }

  /// 保存表单到控制器。
  void _save() {
    final String id = _id.text.trim();
    final String provider = _provider.text.trim();
    final String model = _model.text.trim();

    if (id.isEmpty) {
      _setStatus('服务标识不能为空', isError: true);
      return;
    }
    if (provider.isEmpty) {
      _setStatus('provider 不能为空', isError: true);
      return;
    }

    // 回填过的遮蔽值表示「不改 key」：沿用已保存的那一份。
    final String typedKey = _apiKey.text.trim();
    String apiKey = typedKey;
    if (typedKey == kMaskedApiKey) {
      apiKey = _existingKeyFor(id);
    }

    widget.controller.upsertService(
      LlmService(
        id: id,
        provider: provider,
        model: model,
        baseUrl: _baseUrl.text.trim(),
        apiKey: apiKey,
      ),
    );
    _setStatus('已保存 $id');
  }

  String _existingKeyFor(String id) {
    for (final LlmService service in widget.controller.services) {
      if (service.id == id) {
        return service.apiKey;
      }
    }
    return '';
  }

  void _loadIntoForm(LlmService service) {
    setState(() {
      _editing = service.id;
      _id.text = service.id;
      _provider.text = service.provider;
      _model.text = service.model;
      _baseUrl.text = service.baseUrl;
      _apiKey.text = service.hasApiKey ? kMaskedApiKey : '';
      _revealKey = false;
    });
  }

  void _resetForm() {
    setState(() {
      _editing = null;
      _id.clear();
      _provider.clear();
      _model.clear();
      _baseUrl.clear();
      _apiKey.clear();
      _revealKey = false;
    });
  }

  void _setStatus(String message, {bool isError = false}) {
    setState(() {
      _status = message;
      _statusIsError = isError;
    });
  }
}

/// 头部：标题 + 关闭按钮。
class _Header extends StatelessWidget {
  const _Header({required this.onClose});

  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return Padding(
      padding: const EdgeInsets.fromLTRB(24, 20, 16, 12),
      child: Row(
        children: <Widget>[
          Text(
            '设置',
            style: DswTypography.body.copyWith(
              fontSize: 16,
              fontWeight: FontWeight.w600,
              color: c.labelPrimary,
            ),
          ),
          const Spacer(),
          DswIconButton(
            tooltip: '关闭设置',
            onPressed: onClose,
            size: 30,
            iconSize: 16,
            icon: Icons.close,
          ),
        ],
      ),
    );
  }
}

/// 明文存储的提醒。
class _PlaintextNotice extends StatelessWidget {
  const _PlaintextNotice();

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
      decoration: BoxDecoration(
        color: c.interactiveHover,
        borderRadius: BorderRadius.circular(10),
        border: Border.all(color: c.borderL3),
      ),
      child: Text(
        'API key 以明文保存在本机客户端配置里（当前阶段的安全取舍）。'
        '接入 kb_core 之后会改为由服务端统一保管。',
        style: DswTypography.caption.copyWith(color: c.labelSecondary),
      ),
    );
  }
}

/// 已配置服务的列表。
class _ServiceList extends StatelessWidget {
  const _ServiceList({
    required this.controller,
    required this.onUse,
    required this.onEdit,
    required this.onDelete,
  });

  final AppController controller;
  final ValueChanged<LlmService> onUse;
  final ValueChanged<LlmService> onEdit;
  final ValueChanged<LlmService> onDelete;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    final List<LlmService> services = controller.services;

    if (services.isEmpty) {
      return Container(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 14),
        decoration: BoxDecoration(
          color: c.bgBase,
          borderRadius: BorderRadius.circular(10),
          border: Border.all(color: c.borderL3),
        ),
        child: Text(
          '还没有配置任何服务。填好下面的表单并保存即可。',
          style: DswTypography.caption.copyWith(color: c.labelSecondary),
        ),
      );
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        for (final LlmService service in services) ...<Widget>[
          _ServiceTile(
            service: service,
            active: controller.activeServiceId == service.id,
            onUse: () => onUse(service),
            onEdit: () => onEdit(service),
            onDelete: () => onDelete(service),
          ),
          const SizedBox(height: 8),
        ],
      ],
    );
  }
}

/// 一条服务。
class _ServiceTile extends StatelessWidget {
  const _ServiceTile({
    required this.service,
    required this.active,
    required this.onUse,
    required this.onEdit,
    required this.onDelete,
  });

  final LlmService service;
  final bool active;
  final VoidCallback onUse;
  final VoidCallback onEdit;
  final VoidCallback onDelete;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
      decoration: BoxDecoration(
        color: c.bgBase,
        borderRadius: BorderRadius.circular(10),
        border: Border.all(
          color: active
              ? c.stateBusinessPrimary.withValues(alpha: 0.55)
              : c.borderL3,
        ),
      ),
      child: Row(
        children: <Widget>[
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: <Widget>[
                Row(
                  children: <Widget>[
                    Text(
                      service.id,
                      style: DswTypography.body.copyWith(
                        fontWeight: FontWeight.w500,
                        color: c.labelPrimary,
                      ),
                    ),
                    if (active) ...<Widget>[
                      const SizedBox(width: 8),
                      const _Tag(label: '当前'),
                    ],
                    if (!service.hasApiKey) ...<Widget>[
                      const SizedBox(width: 8),
                      const _Tag(label: '缺少 API key', warn: true),
                    ],
                  ],
                ),
                const SizedBox(height: 2),
                Text(
                  service.summary,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: DswTypography.caption.copyWith(color: c.labelCaption),
                ),
              ],
            ),
          ),
          const SizedBox(width: 12),
          DswMiniButton(
            label: '使用',
            onPressed: active ? null : onUse,
          ),
          const SizedBox(width: 6),
          DswMiniButton(label: '编辑', onPressed: onEdit),
          const SizedBox(width: 6),
          DswMiniButton(label: '删除', onPressed: onDelete, danger: true),
        ],
      ),
    );
  }
}

/// 小圆角标签。
class _Tag extends StatelessWidget {
  const _Tag({required this.label, this.warn = false});

  final String label;
  final bool warn;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    final Color ink = warn ? c.stateWarn : c.labelCaption;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 1),
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(999),
        border: Border.all(
          color: warn ? c.stateWarn.withValues(alpha: 0.45) : c.borderL3,
        ),
      ),
      child: Text(
        label,
        style: DswTypography.caption.copyWith(fontSize: 11, color: ink),
      ),
    );
  }
}

/// 主题模式选择。
class _ThemeRow extends StatelessWidget {
  const _ThemeRow({required this.controller});

  final AppController controller;

  @override
  Widget build(BuildContext context) {
    const List<(ThemeMode, String)> options = <(ThemeMode, String)>[
      (ThemeMode.system, '跟随系统'),
      (ThemeMode.dark, '深色'),
      (ThemeMode.light, '浅色'),
    ];

    return Row(
      children: <Widget>[
        for (final (ThemeMode mode, String label) in options) ...<Widget>[
          DswMiniButton(
            label: label,
            onPressed: controller.themeMode == mode
                ? null
                : () => controller.setThemeMode(mode),
          ),
          const SizedBox(width: 8),
        ],
      ],
    );
  }
}

/// 底部状态行，对应网页端的 `#panel-status`。
class _StatusBar extends StatelessWidget {
  const _StatusBar({required this.status, required this.isError});

  final String status;
  final bool isError;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return Padding(
      padding: const EdgeInsets.fromLTRB(24, 8, 24, 16),
      child: SizedBox(
        height: 18,
        child: Text(
          status,
          style: DswTypography.caption.copyWith(
            color: isError ? c.stateError : c.labelSecondary,
          ),
        ),
      ),
    );
  }
}
