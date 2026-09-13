// 底部输入区。
//
// 对齐 DSH 的 `InputBar`：一张 22px 圆角的卡片，底色 `--dsw-specific-input-major`，
// 下方右侧是 34px 的圆形发送按钮（`--dsw-alias-button-info-fill`，即那条
// deepseek 蓝）。按键约定与 DSH 的 `keymap.ts` 一致：
//
// - `Enter` 发送；
// - `Shift+Enter` 换行；
// - 输入法组合中的 Enter 不发送。

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../theme/app_theme.dart';
import '../../theme/dsw_tokens.dart';
import '../common/dsw_icons.dart';

/// 输入框区域的可用高度上限（DSH 的 `--dsh-composer-text-max-height`，14 行）。
const double _kComposerMaxTextHeight = 336;

/// 底部输入区。
class Composer extends StatefulWidget {
  /// 构造输入区。
  const Composer({
    super.key,
    required this.hint,
    required this.onSend,
    this.busy = false,
    this.onCancel,
  });

  /// 占位文字。
  final String hint;

  /// 发送回调，参数是去掉首尾空白的正文。
  final ValueChanged<String> onSend;

  /// 是否正在生成（此时发送按钮变成「停止」）。
  final bool busy;

  /// 停止回调。
  final VoidCallback? onCancel;

  @override
  State<Composer> createState() => _ComposerState();
}

class _ComposerState extends State<Composer> {
  final TextEditingController _input = TextEditingController();
  final FocusNode _focus = FocusNode();
  bool _empty = true;

  @override
  void initState() {
    super.initState();
    _input.addListener(() {
      final bool empty = _input.text.trim().isEmpty;
      if (empty != _empty) {
        setState(() => _empty = empty);
      }
    });
  }

  @override
  void dispose() {
    _input.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _submit() {
    final String text = _input.text.trim();
    if (text.isEmpty) {
      return;
    }
    _input.clear();
    widget.onSend(text);
  }

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;

    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 0, 16, 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: <Widget>[
          DecoratedBox(
            decoration: BoxDecoration(
              color: c.inputMajor,
              borderRadius: BorderRadius.circular(22),
              boxShadow: c.elevationSoft,
            ),
            child: Padding(
              padding: const EdgeInsets.fromLTRB(6, 6, 6, 6),
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.end,
                children: <Widget>[
                  Expanded(
                    child: Focus(
                      // `EditableText` 自己没有按键处理，所以 Enter 会一路冒泡到
                      // 这里；返回 handled 即可拦下「换行」。
                      onKeyEvent: (FocusNode node, KeyEvent event) {
                        if (event is! KeyDownEvent) {
                          return KeyEventResult.ignored;
                        }
                        if (event.logicalKey != LogicalKeyboardKey.enter &&
                            event.logicalKey != LogicalKeyboardKey.numpadEnter) {
                          return KeyEventResult.ignored;
                        }
                        if (HardwareKeyboard.instance.isShiftPressed) {
                          return KeyEventResult.ignored;
                        }
                        _submit();
                        return KeyEventResult.handled;
                      },
                      child: ConstrainedBox(
                        constraints: const BoxConstraints(
                          maxHeight: _kComposerMaxTextHeight,
                        ),
                        child: TextField(
                          controller: _input,
                          focusNode: _focus,
                          maxLines: null,
                          minLines: 1,
                          keyboardType: TextInputType.multiline,
                          style: DswTypography.body.copyWith(
                            color: c.labelPrimary,
                            height: 24 / 14,
                          ),
                          decoration: InputDecoration(
                            isDense: true,
                            border: InputBorder.none,
                            hintText: widget.hint,
                            hintStyle: DswTypography.body.copyWith(
                              color: c.labelCaption,
                            ),
                            contentPadding: const EdgeInsets.fromLTRB(
                              10,
                              8,
                              8,
                              8,
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                  const SizedBox(width: 8),
                  Padding(
                    padding: const EdgeInsets.only(bottom: 2, right: 2),
                    child: _SendButton(
                      busy: widget.busy,
                      enabled: widget.busy || !_empty,
                      onPressed: widget.busy
                          ? widget.onCancel
                          : (_empty ? null : _submit),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// 34px 的圆形发送 / 停止按钮。
class _SendButton extends StatefulWidget {
  const _SendButton({
    required this.busy,
    required this.enabled,
    required this.onPressed,
  });

  final bool busy;
  final bool enabled;
  final VoidCallback? onPressed;

  @override
  State<_SendButton> createState() => _SendButtonState();
}

class _SendButtonState extends State<_SendButton> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    final bool active = widget.enabled;
    return MouseRegion(
      cursor: active ? SystemMouseCursors.click : SystemMouseCursors.basic,
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: Semantics(
        button: true,
        label: widget.busy ? '停止' : '发送',
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: widget.onPressed,
          child: AnimatedOpacity(
            duration: DswMotion.fast,
            opacity: active ? 1 : 0.4,
            child: AnimatedContainer(
              duration: DswMotion.fast,
              curve: DswMotion.easeInOut,
              width: 34,
              height: 34,
              decoration: BoxDecoration(
                color: _hovered && active ? c.buttonInfoHover : c.buttonInfoFill,
                shape: BoxShape.circle,
              ),
              alignment: Alignment.center,
              child: widget.busy
                  ? const StopSquareIcon(size: 16, color: Color(0xFFFFFFFF))
                  : const SendArrowIcon(size: 16, color: Color(0xFFFFFFFF)),
            ),
          ),
        ),
      ),
    );
  }
}
