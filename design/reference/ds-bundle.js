/* @ds-bundle: {"format":4,"namespace":"STELMARDesignSystem_89726f","components":[{"name":"Badge","sourcePath":"components/core/Badge.jsx"},{"name":"Button","sourcePath":"components/core/Button.jsx"},{"name":"Icon","sourcePath":"components/core/Icon.jsx"},{"name":"IconButton","sourcePath":"components/core/IconButton.jsx"},{"name":"Tag","sourcePath":"components/core/Tag.jsx"},{"name":"Dialog","sourcePath":"components/feedback/Dialog.jsx"},{"name":"Toast","sourcePath":"components/feedback/Toast.jsx"},{"name":"Tooltip","sourcePath":"components/feedback/Tooltip.jsx"},{"name":"Checkbox","sourcePath":"components/forms/Checkbox.jsx"},{"name":"Input","sourcePath":"components/forms/Input.jsx"},{"name":"Radio","sourcePath":"components/forms/Radio.jsx"},{"name":"Select","sourcePath":"components/forms/Select.jsx"},{"name":"Switch","sourcePath":"components/forms/Switch.jsx"},{"name":"Card","sourcePath":"components/layout/Card.jsx"},{"name":"PaperCard","sourcePath":"components/layout/PaperCard.jsx"},{"name":"SessionItem","sourcePath":"components/layout/SessionItem.jsx"},{"name":"Tabs","sourcePath":"components/navigation/Tabs.jsx"},{"name":"Countdown","sourcePath":"components/site/Countdown.jsx"},{"name":"GlassPill","sourcePath":"components/site/GlassPill.jsx"},{"name":"Hero","sourcePath":"components/site/Hero.jsx"},{"name":"SiteHeader","sourcePath":"components/site/SiteHeader.jsx"}],"sourceHashes":{"components/core/Badge.jsx":"85f78ccf6ded","components/core/Button.jsx":"3dffd4b69729","components/core/Icon.jsx":"86c917a7161f","components/core/IconButton.jsx":"041bbcd71b57","components/core/Tag.jsx":"2ed87c6b3445","components/feedback/Dialog.jsx":"5ceb02adba8e","components/feedback/Toast.jsx":"377bd98e10f2","components/feedback/Tooltip.jsx":"340ee138f54a","components/forms/Checkbox.jsx":"28b7bfb2f22f","components/forms/Input.jsx":"b2b51f643c47","components/forms/Radio.jsx":"0ec585f2a5e5","components/forms/Select.jsx":"9012bccfe387","components/forms/Switch.jsx":"0e9786bfe441","components/layout/Card.jsx":"49552ab8a703","components/layout/PaperCard.jsx":"980c01029db1","components/layout/SessionItem.jsx":"4a6517c7bae3","components/navigation/Tabs.jsx":"61d3b41fd2d2","components/site/Countdown.jsx":"863455258b74","components/site/GlassPill.jsx":"bc1e4aa58442","components/site/Hero.jsx":"19bbf7a02a6f","components/site/SiteHeader.jsx":"f94fd4352e75","ui_kits/author-portal/DashboardScreen.jsx":"69f1daa42ce6","ui_kits/author-portal/PortalChrome.jsx":"1827b12d9290","ui_kits/author-portal/SignInScreen.jsx":"e72360bdd748","ui_kits/author-portal/SubmitScreen.jsx":"59f2b9f944d2","ui_kits/website/CallForPapersScreen.jsx":"e022540e8d5e","ui_kits/website/Chrome.jsx":"0839b036a704","ui_kits/website/HomeScreen.jsx":"bb7683561d6e","ui_kits/website/ProgramScreen.jsx":"a67eef045dd8","ui_kits/website/content.js":"dd66fd7ac6df"},"inlinedExternals":[],"unexposedExports":[]} */

(() => {

const __ds_ns = (window.STELMARDesignSystem_89726f = window.STELMARDesignSystem_89726f || {});

const __ds_scope = {};

(__ds_ns.__errors = __ds_ns.__errors || []);

// components/core/Badge.jsx
try { (() => {
function _extends() { return _extends = Object.assign ? Object.assign.bind() : function (n) { for (var e = 1; e < arguments.length; e++) { var t = arguments[e]; for (var r in t) ({}).hasOwnProperty.call(t, r) && (n[r] = t[r]); } return n; }, _extends.apply(null, arguments); }
const T = {
  neutral: ['var(--status-neutral-fg)', 'var(--status-neutral-bg)'],
  success: ['var(--status-success-fg)', 'var(--status-success-bg)'],
  warning: ['var(--status-warning-fg)', 'var(--status-warning-bg)'],
  danger: ['var(--status-danger-fg)', 'var(--status-danger-bg)'],
  info: ['var(--status-info-fg)', 'var(--status-info-bg)'],
  award: ['var(--status-award-fg)', 'var(--status-award-bg)'],
  accent: ['var(--paper-0)', 'var(--orange-600)'],
  ink: ['var(--paper-50)', 'var(--ink-900)']
};
function Badge({
  tone = 'neutral',
  dot,
  children,
  style,
  ...rest
}) {
  const [fg, bg] = T[tone] || T.neutral;
  return /*#__PURE__*/React.createElement("span", _extends({}, rest, {
    style: {
      display: 'inline-flex',
      alignItems: 'center',
      gap: 6,
      height: 22,
      padding: '0 8px',
      borderRadius: 'var(--radius-xs)',
      background: bg,
      color: fg,
      fontFamily: 'var(--font-ui)',
      fontSize: 11,
      fontWeight: 600,
      letterSpacing: '0.06em',
      textTransform: 'uppercase',
      whiteSpace: 'nowrap',
      ...style
    }
  }), dot && /*#__PURE__*/React.createElement("span", {
    style: {
      width: 6,
      height: 6,
      borderRadius: 999,
      background: 'currentColor'
    }
  }), children);
}
Object.assign(__ds_scope, { Badge });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/core/Badge.jsx", error: String((e && e.message) || e) }); }

// components/core/Icon.jsx
try { (() => {
function _extends() { return _extends = Object.assign ? Object.assign.bind() : function (n) { for (var e = 1; e < arguments.length; e++) { var t = arguments[e]; for (var r in t) ({}).hasOwnProperty.call(t, r) && (n[r] = t[r]); } return n; }, _extends.apply(null, arguments); }
const BASE = 'https://unpkg.com/lucide-static@0.460.0/icons/';
function Icon({
  name,
  size = 18,
  color = 'currentColor',
  label,
  style,
  ...rest
}) {
  const url = 'url(' + BASE + name + '.svg) center/contain no-repeat';
  return /*#__PURE__*/React.createElement("span", _extends({
    role: label ? 'img' : undefined,
    "aria-label": label,
    "aria-hidden": label ? undefined : true
  }, rest, {
    style: {
      display: 'inline-block',
      width: size,
      height: size,
      flex: 'none',
      backgroundColor: color,
      WebkitMask: url,
      mask: url,
      ...style
    }
  }));
}
Object.assign(__ds_scope, { Icon });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/core/Icon.jsx", error: String((e && e.message) || e) }); }

// components/core/Button.jsx
try { (() => {
function _extends() { return _extends = Object.assign ? Object.assign.bind() : function (n) { for (var e = 1; e < arguments.length; e++) { var t = arguments[e]; for (var r in t) ({}).hasOwnProperty.call(t, r) && (n[r] = t[r]); } return n; }, _extends.apply(null, arguments); }
const SIZES = {
  sm: {
    h: 32,
    px: 12,
    fs: 13,
    ic: 16
  },
  md: {
    h: 40,
    px: 16,
    fs: 15,
    ic: 18
  },
  lg: {
    h: 54,
    px: 30,
    fs: 19,
    ic: 20
  }
};
const V = {
  primary: {
    bg: 'var(--action-primary)',
    hov: 'var(--action-primary-hover)',
    prs: 'var(--action-primary-press)',
    fg: 'var(--paper-50)',
    bd: 'transparent'
  },
  accent: {
    bg: 'var(--action-accent)',
    hov: 'var(--action-accent-hover)',
    prs: 'var(--action-accent-press)',
    fg: 'var(--paper-0)',
    bd: 'transparent'
  },
  secondary: {
    bg: 'var(--paper-0)',
    hov: 'var(--paper-100)',
    prs: 'var(--paper-200)',
    fg: 'var(--ink-900)',
    bd: 'var(--ink-900)'
  },
  ghost: {
    bg: 'transparent',
    hov: 'var(--ink-100)',
    prs: 'var(--ink-200)',
    fg: 'var(--ink-900)',
    bd: 'transparent'
  },
  light: {
    bg: 'var(--action-light)',
    hov: 'var(--action-light-hover)',
    prs: 'var(--ink-400)',
    fg: 'var(--ink-900)',
    bd: 'transparent'
  },
  glass: {
    bg: 'var(--glass-bg)',
    hov: 'var(--glass-bg-strong)',
    prs: 'var(--glass-bg-strong)',
    fg: 'var(--paper-0)',
    bd: 'var(--glass-border)'
  },
  inverse: {
    bg: 'var(--paper-50)',
    hov: 'var(--paper-200)',
    prs: 'var(--paper-300)',
    fg: 'var(--ink-900)',
    bd: 'transparent'
  }
};
function Button({
  variant = 'primary',
  size = 'md',
  iconLeft,
  iconRight,
  fullWidth,
  disabled,
  type = 'button',
  children,
  style,
  ...rest
}) {
  const [hover, setHover] = React.useState(false);
  const [press, setPress] = React.useState(false);
  const s = SIZES[size] || SIZES.md,
    v = V[variant] || V.primary;
  const bg = disabled ? variant === 'ghost' ? 'transparent' : 'var(--ink-100)' : press ? v.prs : hover ? v.hov : v.bg;
  return /*#__PURE__*/React.createElement("button", _extends({
    type: type,
    disabled: disabled
  }, rest, {
    onMouseEnter: () => setHover(true),
    onMouseLeave: () => {
      setHover(false);
      setPress(false);
    },
    onMouseDown: () => setPress(true),
    onMouseUp: () => setPress(false),
    style: {
      display: 'inline-flex',
      alignItems: 'center',
      justifyContent: 'center',
      gap: 8,
      height: s.h,
      padding: '0 ' + s.px + 'px',
      width: fullWidth ? '100%' : undefined,
      fontFamily: 'var(--font-ui)',
      fontSize: s.fs,
      fontWeight: 500,
      whiteSpace: 'nowrap',
      background: bg,
      color: disabled ? 'var(--text-disabled)' : v.fg,
      border: '1px solid ' + (disabled ? 'var(--ink-200)' : v.bd),
      borderRadius: 'var(--radius-sm)',
      cursor: disabled ? 'not-allowed' : 'pointer',
      transition: 'background var(--dur-fast) var(--ease-standard)',
      outline: 'none',
      ...style
    }
  }), iconLeft && /*#__PURE__*/React.createElement(__ds_scope.Icon, {
    name: iconLeft,
    size: s.ic
  }), children, iconRight && /*#__PURE__*/React.createElement(__ds_scope.Icon, {
    name: iconRight,
    size: s.ic
  }));
}
Object.assign(__ds_scope, { Button });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/core/Button.jsx", error: String((e && e.message) || e) }); }

// components/core/IconButton.jsx
try { (() => {
function _extends() { return _extends = Object.assign ? Object.assign.bind() : function (n) { for (var e = 1; e < arguments.length; e++) { var t = arguments[e]; for (var r in t) ({}).hasOwnProperty.call(t, r) && (n[r] = t[r]); } return n; }, _extends.apply(null, arguments); }
function IconButton({
  icon,
  label,
  variant = 'ghost',
  size = 'md',
  disabled,
  style,
  ...rest
}) {
  const [hover, setHover] = React.useState(false);
  const d = {
    sm: 32,
    md: 40,
    lg: 48
  }[size] || 40;
  const fill = variant === 'primary',
    outline = variant === 'secondary';
  const bg = fill ? hover ? 'var(--action-primary-hover)' : 'var(--action-primary)' : hover ? 'var(--ink-100)' : outline ? 'var(--paper-0)' : 'transparent';
  return /*#__PURE__*/React.createElement("button", _extends({
    type: "button",
    "aria-label": label,
    title: label,
    disabled: disabled
  }, rest, {
    onMouseEnter: () => setHover(true),
    onMouseLeave: () => setHover(false),
    style: {
      width: d,
      height: d,
      display: 'inline-flex',
      alignItems: 'center',
      justifyContent: 'center',
      padding: 0,
      borderRadius: 'var(--radius-sm)',
      background: disabled ? 'transparent' : bg,
      color: disabled ? 'var(--text-disabled)' : fill ? 'var(--paper-50)' : 'var(--ink-900)',
      border: '1px solid ' + (outline ? 'var(--ink-300)' : 'transparent'),
      cursor: disabled ? 'not-allowed' : 'pointer',
      transition: 'background var(--dur-fast) var(--ease-standard)',
      ...style
    }
  }), /*#__PURE__*/React.createElement(__ds_scope.Icon, {
    name: icon,
    size: size === 'sm' ? 16 : 18
  }));
}
Object.assign(__ds_scope, { IconButton });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/core/IconButton.jsx", error: String((e && e.message) || e) }); }

// components/core/Tag.jsx
try { (() => {
function _extends() { return _extends = Object.assign ? Object.assign.bind() : function (n) { for (var e = 1; e < arguments.length; e++) { var t = arguments[e]; for (var r in t) ({}).hasOwnProperty.call(t, r) && (n[r] = t[r]); } return n; }, _extends.apply(null, arguments); }
function Tag({
  selected,
  onRemove,
  onClick,
  children,
  style,
  ...rest
}) {
  const [hover, setHover] = React.useState(false);
  const interactive = !!onClick;
  return /*#__PURE__*/React.createElement("span", _extends({}, rest, {
    onClick: onClick,
    onMouseEnter: () => setHover(true),
    onMouseLeave: () => setHover(false),
    style: {
      display: 'inline-flex',
      alignItems: 'center',
      gap: 6,
      height: 28,
      padding: onRemove ? '0 6px 0 12px' : '0 12px',
      borderRadius: 'var(--radius-pill)',
      border: '1px solid ' + (selected ? 'var(--ink-900)' : 'var(--ink-300)'),
      background: selected ? 'var(--ink-900)' : hover && interactive ? 'var(--paper-100)' : 'var(--paper-0)',
      color: selected ? 'var(--paper-50)' : 'var(--ink-800)',
      fontFamily: 'var(--font-ui)',
      fontSize: 13,
      cursor: interactive ? 'pointer' : 'default',
      userSelect: 'none',
      transition: 'background var(--dur-fast) var(--ease-standard)',
      ...style
    }
  }), children, onRemove && /*#__PURE__*/React.createElement("span", {
    role: "button",
    "aria-label": "Remove",
    onClick: e => {
      e.stopPropagation();
      onRemove();
    },
    style: {
      display: 'inline-flex',
      cursor: 'pointer',
      opacity: .7
    }
  }, /*#__PURE__*/React.createElement(__ds_scope.Icon, {
    name: "x",
    size: 14
  })));
}
Object.assign(__ds_scope, { Tag });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/core/Tag.jsx", error: String((e && e.message) || e) }); }

// components/feedback/Dialog.jsx
try { (() => {
function Dialog({
  open,
  onClose,
  title,
  eyebrow,
  children,
  footer,
  width = 520
}) {
  if (!open) return null;
  return /*#__PURE__*/React.createElement("div", {
    onClick: onClose,
    style: {
      position: 'fixed',
      inset: 0,
      background: 'var(--scrim)',
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      zIndex: 1000,
      padding: 24
    }
  }, /*#__PURE__*/React.createElement("div", {
    role: "dialog",
    "aria-modal": "true",
    onClick: e => e.stopPropagation(),
    style: {
      width: '100%',
      maxWidth: width,
      background: 'var(--paper-0)',
      borderRadius: 'var(--radius-lg)',
      boxShadow: 'var(--shadow-3)',
      borderTop: '3px solid var(--ink-900)',
      overflow: 'hidden'
    }
  }, /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      justifyContent: 'space-between',
      alignItems: 'flex-start',
      gap: 16,
      padding: '20px 24px 0'
    }
  }, /*#__PURE__*/React.createElement("div", null, eyebrow && /*#__PURE__*/React.createElement("div", {
    style: {
      fontFamily: 'var(--font-ui)',
      fontSize: 12,
      fontWeight: 600,
      letterSpacing: '0.08em',
      textTransform: 'uppercase',
      color: 'var(--ink-600)',
      marginBottom: 6
    }
  }, eyebrow), /*#__PURE__*/React.createElement("div", {
    style: {
      fontFamily: 'var(--font-heading)',
      fontSize: 24,
      lineHeight: '30px',
      fontWeight: 700,
      color: 'var(--ink-900)'
    }
  }, title)), onClose && /*#__PURE__*/React.createElement(__ds_scope.IconButton, {
    icon: "x",
    label: "Close",
    size: "sm",
    onClick: onClose
  })), /*#__PURE__*/React.createElement("div", {
    style: {
      padding: '12px 24px 24px',
      fontFamily: 'var(--font-ui)',
      fontSize: 15,
      lineHeight: '24px',
      color: 'var(--ink-800)'
    }
  }, children), footer && /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      justifyContent: 'flex-end',
      gap: 8,
      padding: '14px 24px',
      background: 'var(--paper-50)',
      borderTop: '1px solid var(--ink-200)'
    }
  }, footer)));
}
Object.assign(__ds_scope, { Dialog });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/feedback/Dialog.jsx", error: String((e && e.message) || e) }); }

// components/feedback/Toast.jsx
try { (() => {
const T = {
  success: ['check-circle', 'var(--green-600)'],
  info: ['info', 'var(--blue-500)'],
  warning: ['alert-triangle', 'var(--amber-600)'],
  danger: ['alert-circle', 'var(--red-600)']
};
function Toast({
  tone = 'success',
  title,
  message,
  action,
  onClose,
  style
}) {
  const [ic, c] = T[tone] || T.info;
  return /*#__PURE__*/React.createElement("div", {
    role: "status",
    style: {
      display: 'flex',
      gap: 12,
      alignItems: 'flex-start',
      width: 380,
      maxWidth: '100%',
      boxSizing: 'border-box',
      padding: '14px 14px 14px 16px',
      background: 'var(--ink-900)',
      color: 'var(--paper-50)',
      borderRadius: 'var(--radius-md)',
      boxShadow: 'var(--shadow-2)',
      ...style
    }
  }, /*#__PURE__*/React.createElement(__ds_scope.Icon, {
    name: ic,
    size: 18,
    color: c,
    style: {
      marginTop: 2,
      filter: 'brightness(1.4)'
    }
  }), /*#__PURE__*/React.createElement("div", {
    style: {
      flex: 1,
      minWidth: 0
    }
  }, title && /*#__PURE__*/React.createElement("div", {
    style: {
      fontFamily: 'var(--font-ui)',
      fontSize: 14,
      fontWeight: 600,
      lineHeight: '22px'
    }
  }, title), message && /*#__PURE__*/React.createElement("div", {
    style: {
      fontFamily: 'var(--font-ui)',
      fontSize: 13,
      lineHeight: '19px',
      color: 'var(--ink-300)'
    }
  }, message), action && /*#__PURE__*/React.createElement("div", {
    style: {
      marginTop: 8
    }
  }, action)), onClose && /*#__PURE__*/React.createElement("button", {
    "aria-label": "Dismiss",
    onClick: onClose,
    style: {
      background: 'none',
      border: 'none',
      color: 'var(--ink-300)',
      cursor: 'pointer',
      padding: 2,
      display: 'flex'
    }
  }, /*#__PURE__*/React.createElement(__ds_scope.Icon, {
    name: "x",
    size: 16
  })));
}
Object.assign(__ds_scope, { Toast });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/feedback/Toast.jsx", error: String((e && e.message) || e) }); }

// components/feedback/Tooltip.jsx
try { (() => {
function Tooltip({
  content,
  placement = 'top',
  children
}) {
  const [show, setShow] = React.useState(false);
  const pos = placement === 'bottom' ? {
    top: 'calc(100% + 6px)'
  } : {
    bottom: 'calc(100% + 6px)'
  };
  return /*#__PURE__*/React.createElement("span", {
    style: {
      position: 'relative',
      display: 'inline-flex'
    },
    onMouseEnter: () => setShow(true),
    onMouseLeave: () => setShow(false),
    onFocus: () => setShow(true),
    onBlur: () => setShow(false)
  }, children, /*#__PURE__*/React.createElement("span", {
    role: "tooltip",
    style: {
      position: 'absolute',
      left: '50%',
      transform: 'translateX(-50%)',
      ...pos,
      whiteSpace: 'nowrap',
      pointerEvents: 'none',
      background: 'var(--ink-900)',
      color: 'var(--paper-50)',
      fontFamily: 'var(--font-ui)',
      fontSize: 12,
      lineHeight: '16px',
      padding: '5px 8px',
      borderRadius: 'var(--radius-xs)',
      opacity: show ? 1 : 0,
      transition: 'opacity var(--dur-fast) var(--ease-standard)',
      zIndex: 50
    }
  }, content));
}
Object.assign(__ds_scope, { Tooltip });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/feedback/Tooltip.jsx", error: String((e && e.message) || e) }); }

// components/forms/Checkbox.jsx
try { (() => {
function Checkbox({
  checked,
  onChange,
  label,
  description,
  disabled,
  style
}) {
  return /*#__PURE__*/React.createElement("label", {
    style: {
      display: 'flex',
      gap: 10,
      alignItems: 'flex-start',
      cursor: disabled ? 'not-allowed' : 'pointer',
      opacity: disabled ? .5 : 1,
      ...style
    }
  }, /*#__PURE__*/React.createElement("input", {
    type: "checkbox",
    checked: !!checked,
    disabled: disabled,
    onChange: e => onChange && onChange(e.target.checked),
    style: {
      position: 'absolute',
      opacity: 0,
      width: 0,
      height: 0
    }
  }), /*#__PURE__*/React.createElement("span", {
    style: {
      width: 18,
      height: 18,
      marginTop: 2,
      flex: 'none',
      boxSizing: 'border-box',
      borderRadius: 'var(--radius-xs)',
      border: '1px solid ' + (checked ? 'var(--ink-900)' : 'var(--ink-400)'),
      background: checked ? 'var(--ink-900)' : 'var(--paper-0)',
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      transition: 'background var(--dur-fast)'
    }
  }, checked && /*#__PURE__*/React.createElement(__ds_scope.Icon, {
    name: "check",
    size: 14,
    color: "var(--paper-50)"
  })), /*#__PURE__*/React.createElement("span", {
    style: {
      display: 'flex',
      flexDirection: 'column',
      gap: 2
    }
  }, /*#__PURE__*/React.createElement("span", {
    style: {
      fontFamily: 'var(--font-ui)',
      fontSize: 14,
      lineHeight: '22px',
      color: 'var(--ink-900)'
    }
  }, label), description && /*#__PURE__*/React.createElement("span", {
    style: {
      fontFamily: 'var(--font-ui)',
      fontSize: 13,
      lineHeight: '18px',
      color: 'var(--ink-600)'
    }
  }, description)));
}
Object.assign(__ds_scope, { Checkbox });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/forms/Checkbox.jsx", error: String((e && e.message) || e) }); }

// components/forms/Input.jsx
try { (() => {
function _extends() { return _extends = Object.assign ? Object.assign.bind() : function (n) { for (var e = 1; e < arguments.length; e++) { var t = arguments[e]; for (var r in t) ({}).hasOwnProperty.call(t, r) && (n[r] = t[r]); } return n; }, _extends.apply(null, arguments); }
const labelStyle = {
  fontFamily: 'var(--font-ui)',
  fontSize: 13,
  fontWeight: 500,
  color: 'var(--ink-900)'
};
const hintStyle = err => ({
  fontFamily: 'var(--font-ui)',
  fontSize: 12,
  lineHeight: '16px',
  color: err ? 'var(--red-600)' : 'var(--ink-500)'
});
function Input({
  label,
  hint,
  error,
  iconLeft,
  multiline,
  rows = 4,
  required,
  disabled,
  id,
  style,
  ...rest
}) {
  const [focus, setFocus] = React.useState(false);
  const fid = id || React.useId();
  const bd = error ? 'var(--red-600)' : focus ? 'var(--blue-600)' : 'var(--ink-300)';
  const Tag = multiline ? 'textarea' : 'input';
  return /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      flexDirection: 'column',
      gap: 6,
      ...style
    }
  }, label && /*#__PURE__*/React.createElement("label", {
    htmlFor: fid,
    style: labelStyle
  }, label, required && /*#__PURE__*/React.createElement("span", {
    style: {
      color: 'var(--orange-600)'
    }
  }, " *")), /*#__PURE__*/React.createElement("div", {
    style: {
      position: 'relative',
      display: 'flex'
    }
  }, iconLeft && /*#__PURE__*/React.createElement(__ds_scope.Icon, {
    name: iconLeft,
    size: 16,
    color: "var(--ink-500)",
    style: {
      position: 'absolute',
      left: 12,
      top: multiline ? 12 : 12
    }
  }), /*#__PURE__*/React.createElement(Tag, _extends({
    id: fid,
    rows: multiline ? rows : undefined,
    disabled: disabled,
    required: required
  }, rest, {
    onFocus: e => {
      setFocus(true);
      rest.onFocus && rest.onFocus(e);
    },
    onBlur: e => {
      setFocus(false);
      rest.onBlur && rest.onBlur(e);
    },
    style: {
      width: '100%',
      boxSizing: 'border-box',
      height: multiline ? 'auto' : 40,
      padding: multiline ? '10px 12px' : '0 12px',
      paddingLeft: iconLeft ? 36 : 12,
      fontFamily: 'var(--font-ui)',
      fontSize: 14,
      lineHeight: '22px',
      color: 'var(--ink-900)',
      background: disabled ? 'var(--paper-100)' : 'var(--paper-0)',
      border: '1px solid ' + bd,
      borderRadius: 'var(--radius-sm)',
      outline: 'none',
      boxShadow: focus ? '0 0 0 3px ' + (error ? 'var(--red-100)' : 'var(--blue-100)') : 'none',
      resize: multiline ? 'vertical' : undefined,
      transition: 'border-color var(--dur-fast), box-shadow var(--dur-fast)'
    }
  }))), (error || hint) && /*#__PURE__*/React.createElement("div", {
    style: hintStyle(!!error)
  }, error || hint));
}
Object.assign(__ds_scope, { Input });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/forms/Input.jsx", error: String((e && e.message) || e) }); }

// components/forms/Radio.jsx
try { (() => {
function Radio({
  checked,
  onChange,
  label,
  description,
  name,
  value,
  disabled,
  style
}) {
  return /*#__PURE__*/React.createElement("label", {
    style: {
      display: 'flex',
      gap: 10,
      alignItems: 'flex-start',
      cursor: disabled ? 'not-allowed' : 'pointer',
      opacity: disabled ? .5 : 1,
      ...style
    }
  }, /*#__PURE__*/React.createElement("input", {
    type: "radio",
    name: name,
    value: value,
    checked: !!checked,
    disabled: disabled,
    onChange: () => onChange && onChange(value),
    style: {
      position: 'absolute',
      opacity: 0,
      width: 0,
      height: 0
    }
  }), /*#__PURE__*/React.createElement("span", {
    style: {
      width: 18,
      height: 18,
      marginTop: 2,
      flex: 'none',
      boxSizing: 'border-box',
      borderRadius: 999,
      border: '1px solid ' + (checked ? 'var(--ink-900)' : 'var(--ink-400)'),
      background: 'var(--paper-0)',
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center'
    }
  }, checked && /*#__PURE__*/React.createElement("span", {
    style: {
      width: 8,
      height: 8,
      borderRadius: 999,
      background: 'var(--ink-900)'
    }
  })), /*#__PURE__*/React.createElement("span", {
    style: {
      display: 'flex',
      flexDirection: 'column',
      gap: 2
    }
  }, /*#__PURE__*/React.createElement("span", {
    style: {
      fontFamily: 'var(--font-ui)',
      fontSize: 14,
      lineHeight: '22px',
      color: 'var(--ink-900)'
    }
  }, label), description && /*#__PURE__*/React.createElement("span", {
    style: {
      fontFamily: 'var(--font-ui)',
      fontSize: 13,
      lineHeight: '18px',
      color: 'var(--ink-600)'
    }
  }, description)));
}
Object.assign(__ds_scope, { Radio });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/forms/Radio.jsx", error: String((e && e.message) || e) }); }

// components/forms/Select.jsx
try { (() => {
function _extends() { return _extends = Object.assign ? Object.assign.bind() : function (n) { for (var e = 1; e < arguments.length; e++) { var t = arguments[e]; for (var r in t) ({}).hasOwnProperty.call(t, r) && (n[r] = t[r]); } return n; }, _extends.apply(null, arguments); }
const labelStyle = {
  fontFamily: 'var(--font-ui)',
  fontSize: 13,
  fontWeight: 500,
  color: 'var(--ink-900)'
};
const hintStyle = err => ({
  fontFamily: 'var(--font-ui)',
  fontSize: 12,
  lineHeight: '16px',
  color: err ? 'var(--red-600)' : 'var(--ink-500)'
});
function Select({
  label,
  hint,
  error,
  options = [],
  placeholder,
  required,
  disabled,
  id,
  style,
  ...rest
}) {
  const [focus, setFocus] = React.useState(false);
  const fid = id || React.useId();
  return /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      flexDirection: 'column',
      gap: 6,
      ...style
    }
  }, label && /*#__PURE__*/React.createElement("label", {
    htmlFor: fid,
    style: labelStyle
  }, label, required && /*#__PURE__*/React.createElement("span", {
    style: {
      color: 'var(--orange-600)'
    }
  }, " *")), /*#__PURE__*/React.createElement("div", {
    style: {
      position: 'relative'
    }
  }, /*#__PURE__*/React.createElement("select", _extends({
    id: fid,
    disabled: disabled,
    required: required
  }, rest, {
    onFocus: () => setFocus(true),
    onBlur: () => setFocus(false),
    style: {
      width: '100%',
      height: 40,
      appearance: 'none',
      WebkitAppearance: 'none',
      padding: '0 36px 0 12px',
      fontFamily: 'var(--font-ui)',
      fontSize: 14,
      color: 'var(--ink-900)',
      background: disabled ? 'var(--paper-100)' : 'var(--paper-0)',
      border: '1px solid ' + (error ? 'var(--red-600)' : focus ? 'var(--blue-600)' : 'var(--ink-300)'),
      borderRadius: 'var(--radius-sm)',
      outline: 'none',
      boxShadow: focus ? '0 0 0 3px var(--blue-100)' : 'none',
      cursor: 'pointer'
    }
  }), placeholder && /*#__PURE__*/React.createElement("option", {
    value: ""
  }, placeholder), options.map(o => {
    const v = typeof o === 'string' ? o : o.value;
    const l = typeof o === 'string' ? o : o.label;
    return /*#__PURE__*/React.createElement("option", {
      key: v,
      value: v
    }, l);
  })), /*#__PURE__*/React.createElement(__ds_scope.Icon, {
    name: "chevron-down",
    size: 16,
    color: "var(--ink-600)",
    style: {
      position: 'absolute',
      right: 12,
      top: 12,
      pointerEvents: 'none'
    }
  })), (error || hint) && /*#__PURE__*/React.createElement("div", {
    style: hintStyle(!!error)
  }, error || hint));
}
Object.assign(__ds_scope, { Select });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/forms/Select.jsx", error: String((e && e.message) || e) }); }

// components/forms/Switch.jsx
try { (() => {
function Switch({
  checked,
  onChange,
  label,
  disabled,
  style
}) {
  return /*#__PURE__*/React.createElement("label", {
    style: {
      display: 'inline-flex',
      gap: 10,
      alignItems: 'center',
      cursor: disabled ? 'not-allowed' : 'pointer',
      opacity: disabled ? .5 : 1,
      ...style
    }
  }, /*#__PURE__*/React.createElement("input", {
    type: "checkbox",
    role: "switch",
    checked: !!checked,
    disabled: disabled,
    onChange: e => onChange && onChange(e.target.checked),
    style: {
      position: 'absolute',
      opacity: 0,
      width: 0,
      height: 0
    }
  }), /*#__PURE__*/React.createElement("span", {
    style: {
      width: 34,
      height: 20,
      borderRadius: 999,
      background: checked ? 'var(--ink-900)' : 'var(--ink-300)',
      position: 'relative',
      transition: 'background var(--dur-base) var(--ease-standard)',
      flex: 'none'
    }
  }, /*#__PURE__*/React.createElement("span", {
    style: {
      position: 'absolute',
      top: 2,
      left: checked ? 16 : 2,
      width: 16,
      height: 16,
      borderRadius: 999,
      background: 'var(--paper-0)',
      boxShadow: 'var(--shadow-1)',
      transition: 'left var(--dur-base) var(--ease-standard)'
    }
  })), label && /*#__PURE__*/React.createElement("span", {
    style: {
      fontFamily: 'var(--font-ui)',
      fontSize: 14,
      color: 'var(--ink-900)'
    }
  }, label));
}
Object.assign(__ds_scope, { Switch });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/forms/Switch.jsx", error: String((e && e.message) || e) }); }

// components/layout/Card.jsx
try { (() => {
function _extends() { return _extends = Object.assign ? Object.assign.bind() : function (n) { for (var e = 1; e < arguments.length; e++) { var t = arguments[e]; for (var r in t) ({}).hasOwnProperty.call(t, r) && (n[r] = t[r]); } return n; }, _extends.apply(null, arguments); }
function Card({
  variant = 'outline',
  padding = 24,
  interactive,
  onClick,
  children,
  style,
  ...rest
}) {
  const [hover, setHover] = React.useState(false);
  const V = {
    outline: {
      background: 'var(--surface-card)',
      border: '1px solid var(--border-hairline)'
    },
    muted: {
      background: 'var(--surface-muted)',
      border: '1px solid transparent'
    },
    ruled: {
      background: 'transparent',
      border: 'none',
      borderTop: '2px solid var(--ink-900)',
      borderRadius: 0
    },
    inverse: {
      background: 'var(--surface-inverse)',
      border: '1px solid transparent',
      color: 'var(--text-inverse)'
    }
  }[variant];
  return /*#__PURE__*/React.createElement("div", _extends({}, rest, {
    onClick: onClick,
    onMouseEnter: () => setHover(true),
    onMouseLeave: () => setHover(false),
    style: {
      borderRadius: 'var(--radius-md)',
      padding,
      boxSizing: 'border-box',
      ...V,
      ...(interactive && hover ? {
        borderColor: 'var(--ink-900)',
        boxShadow: 'var(--shadow-1)'
      } : {}),
      cursor: interactive ? 'pointer' : undefined,
      transition: 'border-color var(--dur-fast) var(--ease-standard), box-shadow var(--dur-fast)',
      ...style
    }
  }), children);
}
Object.assign(__ds_scope, { Card });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/layout/Card.jsx", error: String((e && e.message) || e) }); }

// components/layout/PaperCard.jsx
try { (() => {
const STATUS = {
  draft: ['neutral', 'Draft'],
  submitted: ['info', 'Submitted'],
  review: ['info', 'Under review'],
  revision: ['warning', 'Revision requested'],
  accepted: ['success', 'Accepted'],
  rejected: ['danger', 'Rejected'],
  award: ['award', 'Best paper']
};
function PaperCard({
  id,
  title,
  authors,
  affiliation,
  track,
  status,
  abstract,
  keywords = [],
  actions,
  onClick,
  style
}) {
  const st = STATUS[status];
  return /*#__PURE__*/React.createElement(__ds_scope.Card, {
    interactive: !!onClick,
    onClick: onClick,
    padding: 20,
    style: {
      display: 'flex',
      flexDirection: 'column',
      gap: 10,
      ...style
    }
  }, /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      alignItems: 'center',
      gap: 12,
      justifyContent: 'space-between'
    }
  }, /*#__PURE__*/React.createElement("div", {
    style: {
      fontFamily: 'var(--font-ui)',
      fontSize: 12,
      fontWeight: 600,
      letterSpacing: '0.08em',
      textTransform: 'uppercase',
      color: 'var(--ink-600)'
    }
  }, [id, track].filter(Boolean).join(' · ')), st && /*#__PURE__*/React.createElement(__ds_scope.Badge, {
    tone: st[0],
    dot: true
  }, st[1])), /*#__PURE__*/React.createElement("div", {
    style: {
      fontFamily: 'var(--font-heading)',
      fontSize: 21,
      lineHeight: '27px',
      fontWeight: 700,
      color: 'var(--ink-900)',
      textWrap: 'pretty'
    }
  }, title), (authors || affiliation) && /*#__PURE__*/React.createElement("div", {
    style: {
      fontFamily: 'var(--font-ui)',
      fontSize: 14,
      lineHeight: '20px',
      color: 'var(--ink-700)'
    }
  }, authors, affiliation && /*#__PURE__*/React.createElement("span", {
    style: {
      fontStyle: 'italic',
      color: 'var(--ink-600)'
    }
  }, authors ? ' — ' : '', affiliation)), abstract && /*#__PURE__*/React.createElement("div", {
    style: {
      fontFamily: 'var(--font-ui)',
      fontSize: 14,
      lineHeight: '22px',
      color: 'var(--ink-800)',
      display: '-webkit-box',
      WebkitLineClamp: 3,
      WebkitBoxOrient: 'vertical',
      overflow: 'hidden'
    }
  }, abstract), keywords.length > 0 && /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      flexWrap: 'wrap',
      gap: 6
    }
  }, keywords.map(k => /*#__PURE__*/React.createElement(__ds_scope.Tag, {
    key: k,
    style: {
      height: 24,
      fontSize: 12
    }
  }, k))), actions && /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      gap: 8,
      paddingTop: 6,
      borderTop: '1px solid var(--ink-200)',
      marginTop: 4
    }
  }, actions));
}
Object.assign(__ds_scope, { PaperCard });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/layout/PaperCard.jsx", error: String((e && e.message) || e) }); }

// components/layout/SessionItem.jsx
try { (() => {
function SessionItem({
  start,
  end,
  kind,
  title,
  speakers,
  room,
  saved,
  onToggleSave,
  highlight,
  style
}) {
  return /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'grid',
      gridTemplateColumns: '112px minmax(0,1fr) auto',
      gap: 20,
      padding: '16px 0',
      borderBottom: '1px solid var(--ink-200)',
      alignItems: 'start',
      background: highlight ? 'var(--paper-100)' : undefined,
      ...style
    }
  }, /*#__PURE__*/React.createElement("div", {
    style: {
      fontFamily: 'var(--font-mono)',
      fontSize: 13,
      lineHeight: '22px',
      color: 'var(--ink-900)'
    }
  }, start, end && /*#__PURE__*/React.createElement("span", {
    style: {
      color: 'var(--ink-500)'
    }
  }, /*#__PURE__*/React.createElement("br", null), end)), /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      flexDirection: 'column',
      gap: 4,
      minWidth: 0
    }
  }, kind && /*#__PURE__*/React.createElement("div", null, /*#__PURE__*/React.createElement(__ds_scope.Badge, {
    tone: kind === 'Keynote' ? 'ink' : kind === 'Award' ? 'award' : 'neutral'
  }, kind)), /*#__PURE__*/React.createElement("div", {
    style: {
      fontFamily: 'var(--font-heading)',
      fontSize: 19,
      lineHeight: '25px',
      fontWeight: 700,
      color: 'var(--ink-900)',
      textWrap: 'pretty'
    }
  }, title), (speakers || room) && /*#__PURE__*/React.createElement("div", {
    style: {
      fontFamily: 'var(--font-ui)',
      fontSize: 13,
      lineHeight: '20px',
      color: 'var(--ink-600)'
    }
  }, speakers, speakers && room ? ' · ' : '', room)), onToggleSave && /*#__PURE__*/React.createElement(__ds_scope.IconButton, {
    icon: saved ? 'bookmark-check' : 'bookmark',
    label: saved ? 'Remove from my programme' : 'Save to my programme',
    size: "sm",
    onClick: onToggleSave,
    style: {
      color: saved ? 'var(--orange-600)' : undefined
    }
  }));
}
Object.assign(__ds_scope, { SessionItem });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/layout/SessionItem.jsx", error: String((e && e.message) || e) }); }

// components/navigation/Tabs.jsx
try { (() => {
function Tabs({
  items = [],
  value,
  onChange,
  size = 'md',
  style
}) {
  return /*#__PURE__*/React.createElement("div", {
    role: "tablist",
    style: {
      display: 'flex',
      gap: 28,
      borderBottom: '1px solid var(--ink-200)',
      ...style
    }
  }, items.map(it => {
    const v = typeof it === 'string' ? it : it.value,
      l = typeof it === 'string' ? it : it.label,
      meta = typeof it === 'string' ? null : it.meta;
    const on = v === value;
    return /*#__PURE__*/React.createElement("button", {
      key: v,
      role: "tab",
      "aria-selected": on,
      onClick: () => onChange && onChange(v),
      style: {
        background: 'none',
        border: 'none',
        padding: size === 'lg' ? '12px 0 14px' : '10px 0 12px',
        marginBottom: -1,
        cursor: 'pointer',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'flex-start',
        gap: 2,
        borderBottom: '2px solid ' + (on ? 'var(--orange-600)' : 'transparent'),
        color: on ? 'var(--ink-900)' : 'var(--ink-600)',
        transition: 'color var(--dur-fast), border-color var(--dur-base) var(--ease-standard)'
      }
    }, /*#__PURE__*/React.createElement("span", {
      style: {
        fontFamily: 'var(--font-ui)',
        fontSize: size === 'lg' ? 16 : 14,
        fontWeight: on ? 600 : 500
      }
    }, l), meta && /*#__PURE__*/React.createElement("span", {
      style: {
        fontFamily: 'var(--font-ui)',
        fontSize: 12,
        fontWeight: 600,
        letterSpacing: '0.08em',
        textTransform: 'uppercase',
        color: 'var(--ink-500)'
      }
    }, meta));
  }));
}
Object.assign(__ds_scope, { Tabs });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/navigation/Tabs.jsx", error: String((e && e.message) || e) }); }

// components/site/Countdown.jsx
try { (() => {
function Countdown({
  to,
  labels = ['Days', 'Hours', 'Minutes', 'Seconds'],
  tone = 'glass',
  style
}) {
  const target = typeof to === 'number' ? to : new Date(to).getTime();
  const [now, setNow] = React.useState(Date.now());
  React.useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);
  let s = Math.max(0, Math.floor((target - now) / 1000));
  const d = Math.floor(s / 86400);
  s %= 86400;
  const h = Math.floor(s / 3600);
  s %= 3600;
  const m = Math.floor(s / 60);
  const parts = [d, h, m, s % 60].map((n, i) => String(n).padStart(2, '0'));
  const glass = tone === 'glass';
  const tile = {
    minWidth: 48,
    height: 60,
    padding: '0 6px',
    boxSizing: 'border-box',
    borderRadius: 'var(--radius-md)',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    fontFamily: 'var(--font-ui)',
    fontSize: 32,
    fontWeight: 700,
    background: glass ? 'var(--glass-bg-strong)' : 'var(--paper-0)',
    border: '1px solid ' + (glass ? 'var(--glass-border)' : 'var(--ink-200)'),
    color: glass ? 'var(--paper-0)' : 'var(--ink-900)'
  };
  return /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'inline-flex',
      gap: 16,
      padding: '12px 16px',
      borderRadius: 'var(--radius-lg)',
      background: glass ? 'var(--glass-bg)' : 'var(--paper-50)',
      border: '1px solid ' + (glass ? 'var(--glass-border)' : 'var(--ink-200)'),
      backdropFilter: glass ? 'var(--glass-blur)' : undefined,
      WebkitBackdropFilter: glass ? 'var(--glass-blur)' : undefined,
      ...style
    }
  }, parts.map((p, i) => /*#__PURE__*/React.createElement("div", {
    key: i,
    style: {
      display: 'flex',
      flexDirection: 'column',
      alignItems: 'center',
      gap: 6
    }
  }, /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      gap: 6
    }
  }, p.split('').map((c, j) => /*#__PURE__*/React.createElement("span", {
    key: j,
    style: tile
  }, c))), labels && /*#__PURE__*/React.createElement("span", {
    style: {
      fontFamily: 'var(--font-ui)',
      fontSize: 11,
      fontWeight: 600,
      letterSpacing: '0.08em',
      textTransform: 'uppercase',
      color: glass ? 'var(--ink-200)' : 'var(--ink-600)'
    }
  }, labels[i]))));
}
Object.assign(__ds_scope, { Countdown });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/site/Countdown.jsx", error: String((e && e.message) || e) }); }

// components/site/GlassPill.jsx
try { (() => {
function GlassPill({
  icon,
  size = 'md',
  children,
  style
}) {
  const s = {
    sm: [8, 18, 15, 22],
    md: [12, 26, 20, 28],
    lg: [14, 30, 25, 34]
  }[size] || [12, 26, 20, 28];
  return /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'inline-flex',
      alignItems: 'center',
      gap: 10,
      padding: s[0] + 'px ' + s[1] + 'px',
      borderRadius: 'var(--radius-pill)',
      background: 'var(--glass-bg)',
      border: '1px solid var(--glass-border)',
      backdropFilter: 'var(--glass-blur)',
      WebkitBackdropFilter: 'var(--glass-blur)',
      color: 'var(--paper-0)',
      fontFamily: 'var(--font-ui)',
      fontSize: s[2],
      fontWeight: 500,
      lineHeight: s[3] + 'px',
      ...style
    }
  }, icon && /*#__PURE__*/React.createElement(__ds_scope.Icon, {
    name: icon,
    size: Math.round(s[2] * 0.9)
  }), children);
}
Object.assign(__ds_scope, { GlassPill });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/site/GlassPill.jsx", error: String((e && e.message) || e) }); }

// components/site/Hero.jsx
try { (() => {
function Hero({
  image,
  eyebrow,
  title,
  subtitle,
  align = 'center',
  minHeight = 560,
  children,
  style
}) {
  const c = align === 'center';
  return /*#__PURE__*/React.createElement("section", {
    style: {
      position: 'relative',
      minHeight,
      display: 'flex',
      alignItems: 'center',
      justifyContent: c ? 'center' : 'flex-start',
      overflow: 'hidden',
      background: 'var(--ink-800)',
      ...style
    }
  }, image && /*#__PURE__*/React.createElement("div", {
    style: {
      position: 'absolute',
      inset: -12,
      backgroundImage: 'url(' + image + ')',
      backgroundSize: 'cover',
      backgroundPosition: 'center',
      filter: 'blur(6px)'
    }
  }), /*#__PURE__*/React.createElement("div", {
    style: {
      position: 'absolute',
      inset: 0,
      background: 'var(--hero-overlay)'
    }
  }), /*#__PURE__*/React.createElement("div", {
    style: {
      position: 'relative',
      width: '100%',
      maxWidth: 1100,
      boxSizing: 'border-box',
      padding: 'clamp(48px, 8vw, 96px) clamp(20px, 4vw, 48px)',
      textAlign: c ? 'center' : 'left',
      display: 'flex',
      flexDirection: 'column',
      alignItems: c ? 'center' : 'flex-start',
      gap: 24
    }
  }, eyebrow && /*#__PURE__*/React.createElement("div", {
    style: {
      fontFamily: 'var(--font-ui)',
      fontSize: 13,
      fontWeight: 600,
      letterSpacing: '0.08em',
      textTransform: 'uppercase',
      color: 'var(--ink-200)'
    }
  }, eyebrow), title && /*#__PURE__*/React.createElement("h1", {
    style: {
      margin: 0,
      color: 'var(--paper-0)',
      fontFamily: 'var(--font-display)',
      fontWeight: 700,
      fontSize: 'clamp(34px, 4.4vw, 60px)',
      lineHeight: 1.25,
      letterSpacing: '-0.01em',
      textWrap: 'balance'
    }
  }, title), subtitle && /*#__PURE__*/React.createElement("p", {
    style: {
      margin: 0,
      color: 'var(--ink-200)',
      fontSize: 19,
      lineHeight: '30px',
      maxWidth: 720
    }
  }, subtitle), children));
}
Object.assign(__ds_scope, { Hero });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/site/Hero.jsx", error: String((e && e.message) || e) }); }

// components/site/SiteHeader.jsx
try { (() => {
function SiteHeader({
  logo,
  links = [],
  active,
  onNavigate,
  actions,
  sticky = true,
  style
}) {
  return /*#__PURE__*/React.createElement("header", {
    style: {
      background: 'var(--surface-header)',
      position: sticky ? 'sticky' : 'relative',
      top: 0,
      zIndex: 10,
      ...style
    }
  }, /*#__PURE__*/React.createElement("div", {
    style: {
      maxWidth: 1440,
      margin: '0 auto',
      padding: '12px clamp(20px, 4vw, 56px)',
      minHeight: 80,
      boxSizing: 'border-box',
      display: 'flex',
      alignItems: 'center',
      gap: 32
    }
  }, /*#__PURE__*/React.createElement("div", {
    style: {
      flex: 'none',
      cursor: onNavigate ? 'pointer' : undefined
    },
    onClick: () => onNavigate && onNavigate(null)
  }, logo), /*#__PURE__*/React.createElement("nav", {
    style: {
      display: 'flex',
      columnGap: 'clamp(14px, 2vw, 32px)',
      rowGap: 8,
      marginLeft: 'auto',
      flexWrap: 'wrap',
      justifyContent: 'flex-end',
      alignItems: 'center'
    }
  }, links.map(l => {
    const v = typeof l === 'string' ? l : l.value,
      t = typeof l === 'string' ? l : l.label;
    const on = v === active;
    return /*#__PURE__*/React.createElement("a", {
      key: v,
      onClick: () => onNavigate && onNavigate(v),
      style: {
        cursor: 'pointer',
        textDecoration: 'none',
        whiteSpace: 'nowrap',
        fontFamily: 'var(--font-ui)',
        fontSize: 'clamp(13px, 1.1vw, 16px)',
        fontWeight: 500,
        textTransform: 'uppercase',
        color: 'var(--paper-0)',
        borderBottom: '2px solid ' + (on ? 'var(--orange-500)' : 'transparent'),
        paddingBottom: 2
      }
    }, t);
  }), actions)));
}
Object.assign(__ds_scope, { SiteHeader });
})(); } catch (e) { __ds_ns.__errors.push({ path: "components/site/SiteHeader.jsx", error: String((e && e.message) || e) }); }

// ui_kits/author-portal/DashboardScreen.jsx
try { (() => {
function _extends() { return _extends = Object.assign ? Object.assign.bind() : function (n) { for (var e = 1; e < arguments.length; e++) { var t = arguments[e]; for (var r in t) ({}).hasOwnProperty.call(t, r) && (n[r] = t[r]); } return n; }, _extends.apply(null, arguments); }
function DashboardScreen({
  papers,
  go
}) {
  const {
    Tabs,
    PaperCard,
    Button,
    Card,
    Badge
  } = window.STELMARDesignSystem_89726f;
  const [tab, setTab] = React.useState('all');
  const list = papers.filter(p => tab === 'all' || (tab === 'active' ? ['draft', 'submitted', 'review', 'revision'].includes(p.status) : ['accepted', 'rejected', 'award'].includes(p.status)));
  return /*#__PURE__*/React.createElement("main", {
    style: {
      maxWidth: 1120,
      margin: '0 auto',
      padding: '40px 32px 80px',
      display: 'grid',
      gridTemplateColumns: 'minmax(0,1fr) 300px',
      gap: 40
    }
  }, /*#__PURE__*/React.createElement("div", null, /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      justifyContent: 'space-between',
      alignItems: 'flex-end',
      gap: 16,
      marginBottom: 24
    }
  }, /*#__PURE__*/React.createElement("div", null, /*#__PURE__*/React.createElement(PortalOverline, null, "Welcome back, Adaeze"), /*#__PURE__*/React.createElement("h1", {
    style: {
      fontFamily: 'var(--font-heading)',
      fontWeight: 700,
      fontSize: 40,
      lineHeight: '46px',
      margin: '6px 0 0'
    }
  }, "My submissions")), /*#__PURE__*/React.createElement(Button, {
    variant: "accent",
    iconLeft: "plus",
    onClick: () => go('submit')
  }, "New submission")), /*#__PURE__*/React.createElement(Tabs, {
    value: tab,
    onChange: setTab,
    items: [{
      value: 'all',
      label: 'All (' + papers.length + ')'
    }, {
      value: 'active',
      label: 'In progress'
    }, {
      value: 'decided',
      label: 'Decided'
    }]
  }), /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      flexDirection: 'column',
      gap: 16,
      marginTop: 24
    }
  }, list.map(p => /*#__PURE__*/React.createElement(PaperCard, _extends({
    key: p.id
  }, p, {
    actions: /*#__PURE__*/React.createElement(React.Fragment, null, /*#__PURE__*/React.createElement(Button, {
      size: "sm",
      variant: "secondary"
    }, p.status === 'draft' ? 'Continue editing' : p.status === 'revision' ? 'Upload revision' : 'View submission'), ['review', 'revision', 'accepted', 'rejected'].includes(p.status) && /*#__PURE__*/React.createElement(Button, {
      size: "sm",
      variant: "ghost",
      iconLeft: "message-square"
    }, "Reviews (3)"))
  }))))), /*#__PURE__*/React.createElement("aside", {
    style: {
      display: 'flex',
      flexDirection: 'column',
      gap: 16,
      paddingTop: 70
    }
  }, /*#__PURE__*/React.createElement(Card, {
    variant: "muted"
  }, /*#__PURE__*/React.createElement(PortalOverline, null, "Upcoming"), [['1 Feb', 'Full-paper deadline', true], ['18 Mar', 'Rebuttal window opens'], ['28 Mar', 'Notification']].map(([d, l, hot]) => /*#__PURE__*/React.createElement("div", {
    key: l,
    style: {
      display: 'flex',
      gap: 12,
      padding: '10px 0',
      borderBottom: '1px solid var(--ink-200)',
      fontSize: 14
    }
  }, /*#__PURE__*/React.createElement("span", {
    style: {
      fontFamily: 'var(--font-mono)',
      fontSize: 13,
      width: 52,
      color: hot ? 'var(--orange-600)' : 'var(--ink-900)'
    }
  }, d), /*#__PURE__*/React.createElement("span", {
    style: {
      fontWeight: hot ? 600 : 400
    }
  }, l)))), /*#__PURE__*/React.createElement(Card, null, /*#__PURE__*/React.createElement(PortalOverline, null, "Author resources"), /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      flexDirection: 'column',
      gap: 8,
      marginTop: 12,
      fontSize: 14
    }
  }, /*#__PURE__*/React.createElement("a", null, "Formatting guidelines"), /*#__PURE__*/React.createElement("a", null, "Anonymisation checklist"), /*#__PURE__*/React.createElement("a", null, "Conflict-of-interest policy")))));
}
window.DashboardScreen = DashboardScreen;
})(); } catch (e) { __ds_ns.__errors.push({ path: "ui_kits/author-portal/DashboardScreen.jsx", error: String((e && e.message) || e) }); }

// ui_kits/author-portal/PortalChrome.jsx
try { (() => {
function PortalHeader({
  user,
  onSignOut,
  go
}) {
  const {
    IconButton,
    Tooltip
  } = window.STELMARDesignSystem_89726f;
  return /*#__PURE__*/React.createElement("header", {
    style: {
      background: 'var(--surface-header)',
      color: 'var(--paper-50)'
    }
  }, /*#__PURE__*/React.createElement("div", {
    style: {
      maxWidth: 1120,
      margin: '0 auto',
      padding: '0 32px',
      height: 72,
      display: 'flex',
      alignItems: 'center',
      gap: 20
    }
  }, /*#__PURE__*/React.createElement("a", {
    onClick: () => go('dashboard'),
    style: {
      cursor: 'pointer',
      textDecoration: 'none',
      display: 'flex',
      alignItems: 'center',
      gap: 16
    }
  }, /*#__PURE__*/React.createElement("img", {
    src: "../../assets/logo-lockup-header.png",
    alt: "STELMAR 2026",
    style: {
      height: 52,
      display: 'block'
    }
  }), /*#__PURE__*/React.createElement("span", {
    style: {
      fontFamily: 'var(--font-mono)',
      fontSize: 10,
      letterSpacing: '0.16em',
      color: 'var(--ink-300)',
      textTransform: 'uppercase'
    }
  }, "Author portal \xB7 2026")), /*#__PURE__*/React.createElement("div", {
    style: {
      marginLeft: 'auto',
      display: 'flex',
      alignItems: 'center',
      gap: 12
    }
  }, /*#__PURE__*/React.createElement(Tooltip, {
    content: "Notifications",
    placement: "bottom"
  }, /*#__PURE__*/React.createElement(IconButton, {
    icon: "bell",
    label: "Notifications",
    style: {
      color: 'var(--paper-50)'
    }
  })), /*#__PURE__*/React.createElement("div", {
    style: {
      width: 32,
      height: 32,
      borderRadius: 999,
      background: 'var(--paper-200)',
      color: 'var(--ink-900)',
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      fontFamily: 'var(--font-ui)',
      fontSize: 13,
      fontWeight: 600
    }
  }, user.initials), /*#__PURE__*/React.createElement("a", {
    onClick: onSignOut,
    style: {
      cursor: 'pointer',
      color: 'var(--ink-300)',
      fontSize: 13
    }
  }, "Sign out"))));
}
function PortalOverline({
  children,
  color
}) {
  return /*#__PURE__*/React.createElement("div", {
    style: {
      fontFamily: 'var(--font-ui)',
      fontSize: 12,
      fontWeight: 600,
      letterSpacing: '0.08em',
      textTransform: 'uppercase',
      color: color || 'var(--ink-600)',
      fontWeight: 500
    }
  }, children);
}
Object.assign(window, {
  PortalHeader,
  PortalOverline
});
})(); } catch (e) { __ds_ns.__errors.push({ path: "ui_kits/author-portal/PortalChrome.jsx", error: String((e && e.message) || e) }); }

// ui_kits/author-portal/SignInScreen.jsx
try { (() => {
function SignInScreen({
  onSignIn
}) {
  const {
    Input,
    Button,
    Checkbox,
    Card
  } = window.STELMARDesignSystem_89726f;
  const [email, setEmail] = React.useState('a.okafor@ulisboa.pt');
  const [pw, setPw] = React.useState('••••••••••');
  const [keep, setKeep] = React.useState(true);
  const [err, setErr] = React.useState('');
  const submit = e => {
    e.preventDefault();
    if (!email.includes('@')) {
      setErr('Enter the email you registered with');
      return;
    }
    onSignIn();
  };
  return /*#__PURE__*/React.createElement("div", {
    style: {
      minHeight: '100vh',
      display: 'grid',
      gridTemplateColumns: 'minmax(0,1fr) minmax(0,1fr)'
    }
  }, /*#__PURE__*/React.createElement("div", {
    style: {
      background: 'var(--surface-header)',
      color: 'var(--paper-0)',
      padding: '48px 56px',
      display: 'flex',
      flexDirection: 'column',
      justifyContent: 'space-between'
    }
  }, /*#__PURE__*/React.createElement("img", {
    src: "../../assets/logo-lockup-header.png",
    alt: "STELMAR 2026",
    style: {
      height: 76,
      alignSelf: 'flex-start'
    }
  }), /*#__PURE__*/React.createElement("div", null, /*#__PURE__*/React.createElement(PortalOverline, {
    color: "var(--orange-200)"
  }, "Full-paper deadline"), /*#__PURE__*/React.createElement("div", {
    style: {
      fontFamily: 'var(--font-heading)',
      fontWeight: 700,
      fontSize: 56,
      lineHeight: '60px',
      letterSpacing: '-0.01em',
      margin: '12px 0'
    }
  }, "15 December 2025"), /*#__PURE__*/React.createElement("div", {
    style: {
      fontFamily: 'var(--font-mono)',
      fontSize: 14,
      color: 'var(--ink-300)'
    }
  }, "23:59 IST")), /*#__PURE__*/React.createElement("div", {
    style: {
      fontSize: 13,
      color: 'var(--ink-400)'
    }
  }, "BML Munjal University \xB7 29\u201331 January 2026")), /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      padding: 48
    }
  }, /*#__PURE__*/React.createElement("form", {
    onSubmit: submit,
    style: {
      width: '100%',
      maxWidth: 380,
      display: 'flex',
      flexDirection: 'column',
      gap: 20
    }
  }, /*#__PURE__*/React.createElement("div", null, /*#__PURE__*/React.createElement("h1", {
    style: {
      fontFamily: 'var(--font-heading)',
      fontWeight: 700,
      fontSize: 40,
      lineHeight: '46px',
      margin: '0 0 6px'
    }
  }, "Sign in"), /*#__PURE__*/React.createElement("p", {
    style: {
      margin: 0,
      color: 'var(--ink-600)',
      fontSize: 15
    }
  }, "Submit and track your papers.")), /*#__PURE__*/React.createElement(Input, {
    label: "Email",
    type: "email",
    value: email,
    onChange: e => {
      setEmail(e.target.value);
      setErr('');
    },
    error: err
  }), /*#__PURE__*/React.createElement(Input, {
    label: "Password",
    type: "password",
    value: pw,
    onChange: e => setPw(e.target.value)
  }), /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      justifyContent: 'space-between',
      alignItems: 'center'
    }
  }, /*#__PURE__*/React.createElement(Checkbox, {
    checked: keep,
    onChange: setKeep,
    label: "Keep me signed in"
  }), /*#__PURE__*/React.createElement("a", {
    style: {
      fontSize: 13,
      cursor: 'pointer'
    }
  }, "Forgot password?")), /*#__PURE__*/React.createElement(Button, {
    type: "submit",
    size: "lg",
    fullWidth: true
  }, "Sign in"), /*#__PURE__*/React.createElement("div", {
    style: {
      fontSize: 14,
      color: 'var(--ink-600)',
      textAlign: 'center'
    }
  }, "New author? ", /*#__PURE__*/React.createElement("a", {
    style: {
      cursor: 'pointer'
    }
  }, "Create an account")))));
}
window.SignInScreen = SignInScreen;
})(); } catch (e) { __ds_ns.__errors.push({ path: "ui_kits/author-portal/SignInScreen.jsx", error: String((e && e.message) || e) }); }

// ui_kits/author-portal/SubmitScreen.jsx
try { (() => {
function SubmitScreen({
  go,
  onSubmitted
}) {
  const {
    Input,
    Select,
    Radio,
    Checkbox,
    Button,
    Tag,
    Dialog,
    Card,
    Icon
  } = window.STELMARDesignSystem_89726f;
  const steps = ['Details', 'Authors', 'Files', 'Declarations'];
  const [step, setStep] = React.useState(0);
  const [title, setTitle] = React.useState('Citation graphs as weak supervision for claim verification');
  const [type, setType] = React.useState('full');
  const [kw, setKw] = React.useState(['claim verification', 'citations']);
  const [kwIn, setKwIn] = React.useState('');
  const [d1, setD1] = React.useState(false);
  const [d2, setD2] = React.useState(false);
  const [confirm, setConfirm] = React.useState(false);
  const H = ({
    children
  }) => /*#__PURE__*/React.createElement("h2", {
    style: {
      fontFamily: 'var(--font-heading)',
      fontWeight: 700,
      fontSize: 26,
      lineHeight: '32px',
      margin: '0 0 20px'
    }
  }, children);
  const body = [/*#__PURE__*/React.createElement("div", {
    key: "0",
    style: {
      display: 'flex',
      flexDirection: 'column',
      gap: 20
    }
  }, /*#__PURE__*/React.createElement(H, null, "Paper details"), /*#__PURE__*/React.createElement(Input, {
    label: "Title",
    required: true,
    value: title,
    onChange: e => setTitle(e.target.value),
    hint: title.split(/\s+/).filter(Boolean).length + ' / 20 words'
  }), /*#__PURE__*/React.createElement(Select, {
    label: "Track",
    required: true,
    placeholder: "Choose a track",
    defaultValue: "Engineering",
    options: ['Engineering', 'Law', 'Management', 'Liberal Arts', 'Interdisciplinary']
  }), /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      flexDirection: 'column',
      gap: 10
    }
  }, /*#__PURE__*/React.createElement("div", {
    style: {
      fontSize: 13,
      fontWeight: 500
    }
  }, "Paper type ", /*#__PURE__*/React.createElement("span", {
    style: {
      color: 'var(--orange-600)'
    }
  }, "*")), /*#__PURE__*/React.createElement(Radio, {
    name: "pt",
    value: "full",
    checked: type === 'full',
    onChange: setType,
    label: "Full paper",
    description: "Up to 12 pages plus references"
  }), /*#__PURE__*/React.createElement(Radio, {
    name: "pt",
    value: "short",
    checked: type === 'short',
    onChange: setType,
    label: "Short paper",
    description: "Up to 6 pages plus references"
  })), /*#__PURE__*/React.createElement(Input, {
    label: "Abstract",
    required: true,
    multiline: true,
    rows: 5,
    defaultValue: "We show that citation context provides a cheap, abundant supervision signal for scientific claim verification, matching fully supervised baselines with 8% of the labels."
  }), /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      flexDirection: 'column',
      gap: 8
    }
  }, /*#__PURE__*/React.createElement(Input, {
    label: "Keywords",
    placeholder: "Type a keyword and press Enter",
    value: kwIn,
    onChange: e => setKwIn(e.target.value),
    onKeyDown: e => {
      if (e.key === 'Enter' && kwIn.trim()) {
        e.preventDefault();
        setKw([...kw, kwIn.trim()]);
        setKwIn('');
      }
    }
  }), /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      gap: 6,
      flexWrap: 'wrap'
    }
  }, kw.map(k => /*#__PURE__*/React.createElement(Tag, {
    key: k,
    onRemove: () => setKw(kw.filter(x => x !== k))
  }, k))))), /*#__PURE__*/React.createElement("div", {
    key: "1",
    style: {
      display: 'flex',
      flexDirection: 'column',
      gap: 16
    }
  }, /*#__PURE__*/React.createElement(H, null, "Authors"), [['Adaeze Okafor', 'BML Munjal University', true], ['Lukas Brandt', 'TU Munich']].map(([n, a, c], i) => /*#__PURE__*/React.createElement(Card, {
    key: n,
    padding: 16,
    style: {
      display: 'flex',
      alignItems: 'center',
      gap: 16
    }
  }, /*#__PURE__*/React.createElement("span", {
    style: {
      fontFamily: 'var(--font-mono)',
      fontSize: 13,
      color: 'var(--ink-500)'
    }
  }, i + 1), /*#__PURE__*/React.createElement("div", {
    style: {
      flex: 1
    }
  }, /*#__PURE__*/React.createElement("div", {
    style: {
      fontWeight: 600,
      fontSize: 15
    }
  }, n), /*#__PURE__*/React.createElement("div", {
    style: {
      fontSize: 13,
      color: 'var(--ink-600)',
      fontStyle: 'italic'
    }
  }, a)), c && /*#__PURE__*/React.createElement("span", {
    style: {
      fontFamily: 'var(--font-ui)',
      fontSize: 12,
      fontWeight: 600,
      letterSpacing: '0.08em',
      textTransform: 'uppercase',
      color: 'var(--blue-700)'
    }
  }, "Corresponding"))), /*#__PURE__*/React.createElement(Button, {
    variant: "secondary",
    iconLeft: "user-plus",
    style: {
      alignSelf: 'flex-start'
    }
  }, "Add co-author")), /*#__PURE__*/React.createElement("div", {
    key: "2",
    style: {
      display: 'flex',
      flexDirection: 'column',
      gap: 16
    }
  }, /*#__PURE__*/React.createElement(H, null, "Files"), /*#__PURE__*/React.createElement("div", {
    style: {
      border: '1px dashed var(--ink-400)',
      borderRadius: 4,
      background: 'var(--paper-0)',
      padding: '40px 24px',
      display: 'flex',
      flexDirection: 'column',
      alignItems: 'center',
      gap: 10,
      textAlign: 'center'
    }
  }, /*#__PURE__*/React.createElement(Icon, {
    name: "upload",
    size: 24,
    color: "var(--ink-600)"
  }), /*#__PURE__*/React.createElement("div", {
    style: {
      fontWeight: 500
    }
  }, "Drop your anonymised PDF here"), /*#__PURE__*/React.createElement("div", {
    style: {
      fontSize: 13,
      color: 'var(--ink-600)'
    }
  }, "PDF only \xB7 max 20 MB \xB7 use the STELMAR template"), /*#__PURE__*/React.createElement(Button, {
    size: "sm",
    variant: "secondary"
  }, "Choose file")), /*#__PURE__*/React.createElement(Card, {
    padding: 14,
    style: {
      display: 'flex',
      alignItems: 'center',
      gap: 12
    }
  }, /*#__PURE__*/React.createElement(Icon, {
    name: "file-text",
    size: 20
  }), /*#__PURE__*/React.createElement("div", {
    style: {
      flex: 1
    }
  }, /*#__PURE__*/React.createElement("div", {
    style: {
      fontSize: 14,
      fontWeight: 500
    }
  }, "okafor-claims-anon.pdf"), /*#__PURE__*/React.createElement("div", {
    style: {
      fontFamily: 'var(--font-mono)',
      fontSize: 12,
      color: 'var(--ink-500)'
    }
  }, "11 pages \xB7 1.8 MB")), /*#__PURE__*/React.createElement(Icon, {
    name: "check-circle",
    size: 18,
    color: "var(--green-600)"
  }))), /*#__PURE__*/React.createElement("div", {
    key: "3",
    style: {
      display: 'flex',
      flexDirection: 'column',
      gap: 16
    }
  }, /*#__PURE__*/React.createElement(H, null, "Declarations"), /*#__PURE__*/React.createElement(Checkbox, {
    checked: d1,
    onChange: setD1,
    label: "This work is original and is not under review at another venue"
  }), /*#__PURE__*/React.createElement(Checkbox, {
    checked: d2,
    onChange: setD2,
    label: "The PDF is anonymised for double-blind review",
    description: "Remove author names, affiliations and identifying acknowledgements."
  }))];
  const last = step === steps.length - 1;
  return /*#__PURE__*/React.createElement("main", {
    style: {
      maxWidth: 1120,
      margin: '0 auto',
      padding: '40px 32px 80px',
      display: 'grid',
      gridTemplateColumns: '220px minmax(0,1fr)',
      gap: 48
    }
  }, /*#__PURE__*/React.createElement("aside", null, /*#__PURE__*/React.createElement("a", {
    onClick: () => go('dashboard'),
    style: {
      cursor: 'pointer',
      fontSize: 13,
      display: 'inline-flex',
      gap: 6,
      alignItems: 'center'
    }
  }, /*#__PURE__*/React.createElement(Icon, {
    name: "arrow-left",
    size: 14
  }), "My submissions"), /*#__PURE__*/React.createElement("div", {
    style: {
      marginTop: 24,
      borderTop: '1px solid var(--ink-900)'
    }
  }, steps.map((s, i) => /*#__PURE__*/React.createElement("div", {
    key: s,
    onClick: () => setStep(i),
    style: {
      cursor: 'pointer',
      display: 'flex',
      gap: 12,
      alignItems: 'center',
      padding: '12px 0',
      borderBottom: '1px solid var(--ink-200)'
    }
  }, /*#__PURE__*/React.createElement("span", {
    style: {
      width: 22,
      height: 22,
      borderRadius: 999,
      flex: 'none',
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      fontFamily: 'var(--font-mono)',
      fontSize: 11,
      background: i < step ? 'var(--ink-900)' : 'transparent',
      color: i < step ? 'var(--paper-50)' : i === step ? 'var(--orange-600)' : 'var(--ink-500)',
      border: '1px solid ' + (i < step ? 'var(--ink-900)' : i === step ? 'var(--orange-600)' : 'var(--ink-300)')
    }
  }, i < step ? /*#__PURE__*/React.createElement(Icon, {
    name: "check",
    size: 12
  }) : i + 1), /*#__PURE__*/React.createElement("span", {
    style: {
      fontSize: 14,
      fontWeight: i === step ? 600 : 400,
      color: i === step ? 'var(--ink-900)' : 'var(--ink-600)'
    }
  }, s))))), /*#__PURE__*/React.createElement("div", {
    style: {
      maxWidth: 640
    }
  }, /*#__PURE__*/React.createElement(PortalOverline, {
    color: "var(--orange-600)"
  }, "New submission \xB7 Step ", step + 1, " of ", steps.length), /*#__PURE__*/React.createElement("div", {
    style: {
      marginTop: 16
    }
  }, body[step]), /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      justifyContent: 'space-between',
      marginTop: 32,
      paddingTop: 20,
      borderTop: '1px solid var(--ink-200)'
    }
  }, /*#__PURE__*/React.createElement(Button, {
    variant: "ghost",
    disabled: step === 0,
    onClick: () => setStep(step - 1),
    iconLeft: "arrow-left"
  }, "Back"), /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      gap: 8
    }
  }, /*#__PURE__*/React.createElement(Button, {
    variant: "secondary"
  }, "Save draft"), last ? /*#__PURE__*/React.createElement(Button, {
    variant: "accent",
    disabled: !(d1 && d2),
    onClick: () => setConfirm(true)
  }, "Submit paper") : /*#__PURE__*/React.createElement(Button, {
    onClick: () => setStep(step + 1),
    iconRight: "arrow-right"
  }, "Continue")))), /*#__PURE__*/React.createElement(Dialog, {
    open: confirm,
    onClose: () => setConfirm(false),
    eyebrow: "Full paper \xB7 Engineering",
    title: "Submit for review?",
    footer: /*#__PURE__*/React.createElement(React.Fragment, null, /*#__PURE__*/React.createElement(Button, {
      variant: "ghost",
      onClick: () => setConfirm(false)
    }, "Cancel"), /*#__PURE__*/React.createElement(Button, {
      variant: "accent",
      onClick: () => {
        setConfirm(false);
        onSubmitted(title);
      }
    }, "Submit"))
  }, "You can still replace files and edit metadata until ", /*#__PURE__*/React.createElement("b", null, "15 December 2025, 23:59 IST"), ". After that, the submission is locked for review."));
}
window.SubmitScreen = SubmitScreen;
})(); } catch (e) { __ds_ns.__errors.push({ path: "ui_kits/author-portal/SubmitScreen.jsx", error: String((e && e.message) || e) }); }

// ui_kits/website/CallForPapersScreen.jsx
try { (() => {
function CallForPapersScreen({
  go
}) {
  const {
    Button,
    Card,
    Icon,
    Tooltip
  } = window.STELMARDesignSystem_89726f;
  const toc = ['Scope', 'Paper types', 'Review process', 'Formatting', 'Ethics'];
  const [active, setActive] = React.useState('Scope');
  return /*#__PURE__*/React.createElement("main", {
    style: {
      maxWidth: 1200,
      margin: '0 auto',
      padding: '56px 32px 0',
      display: 'grid',
      gridTemplateColumns: '220px minmax(0,1fr) 300px',
      gap: 48
    }
  }, /*#__PURE__*/React.createElement("aside", {
    style: {
      position: 'sticky',
      top: 140,
      alignSelf: 'start',
      display: 'flex',
      flexDirection: 'column',
      gap: 2,
      borderTop: '1px solid var(--ink-900)',
      paddingTop: 12
    }
  }, /*#__PURE__*/React.createElement(Overline, null, "On this page"), toc.map(t => /*#__PURE__*/React.createElement("a", {
    key: t,
    onClick: () => setActive(t),
    style: {
      cursor: 'pointer',
      textDecoration: 'none',
      fontSize: 14,
      padding: '6px 0 6px 10px',
      marginTop: 4,
      color: active === t ? 'var(--ink-900)' : 'var(--ink-600)',
      fontWeight: active === t ? 600 : 400,
      borderLeft: '2px solid ' + (active === t ? 'var(--orange-600)' : 'var(--ink-200)')
    }
  }, t))), /*#__PURE__*/React.createElement("article", {
    style: {
      maxWidth: '68ch'
    }
  }, /*#__PURE__*/React.createElement(Overline, {
    color: "var(--orange-600)"
  }, "Call for papers \xB7 STELMAR 2026"), /*#__PURE__*/React.createElement("h1", {
    style: {
      fontFamily: 'var(--font-heading)',
      fontWeight: 700,
      fontSize: 56,
      lineHeight: '60px',
      letterSpacing: '-0.01em',
      margin: '12px 0 20px'
    }
  }, "Call for Papers"), /*#__PURE__*/React.createElement("p", {
    style: {
      fontSize: 18,
      lineHeight: '30px',
      color: 'var(--ink-700)'
    }
  }, "We invite submissions of original, unpublished research. Papers are reviewed double-blind by at least three members of the programme committee, and accepted papers appear in the archival proceedings."), /*#__PURE__*/React.createElement("h2", {
    style: {
      fontFamily: 'var(--font-heading)',
      fontWeight: 700,
      fontSize: 30,
      lineHeight: '36px',
      margin: '40px 0 12px'
    }
  }, "Paper types"), /*#__PURE__*/React.createElement("table", {
    style: {
      width: '100%',
      borderCollapse: 'collapse',
      fontSize: 14
    }
  }, /*#__PURE__*/React.createElement("thead", null, /*#__PURE__*/React.createElement("tr", {
    style: {
      borderBottom: '1px solid var(--ink-900)',
      textAlign: 'left'
    }
  }, ['Type', 'Length', 'Presentation'].map(h => /*#__PURE__*/React.createElement("th", {
    key: h,
    style: {
      fontFamily: 'var(--font-ui)',
      fontSize: 12,
      fontWeight: 600,
      letterSpacing: '0.08em',
      textTransform: 'uppercase',
      fontWeight: 500,
      padding: '8px 0',
      color: 'var(--ink-600)'
    }
  }, h)))), /*#__PURE__*/React.createElement("tbody", null, [['Full paper', '12 pages + refs', '20 min talk'], ['Short paper', '6 pages + refs', '10 min talk'], ['Poster / demo', '2 pages', 'Poster session']].map(r => /*#__PURE__*/React.createElement("tr", {
    key: r[0],
    style: {
      borderBottom: '1px solid var(--ink-200)'
    }
  }, r.map((c, i) => /*#__PURE__*/React.createElement("td", {
    key: i,
    style: {
      padding: '12px 0',
      fontWeight: i ? 400 : 600
    }
  }, c)))))), /*#__PURE__*/React.createElement("h2", {
    style: {
      fontFamily: 'var(--font-heading)',
      fontWeight: 700,
      fontSize: 30,
      lineHeight: '36px',
      margin: '40px 0 12px'
    }
  }, "Review process"), /*#__PURE__*/React.createElement("p", {
    style: {
      lineHeight: '26px'
    }
  }, "Each submission receives at least three reviews. Authors may respond to reviews during a five-day rebuttal window before final decisions. See the ", /*#__PURE__*/React.createElement("a", null, "reviewer guidelines"), " for criteria."), /*#__PURE__*/React.createElement("blockquote", {
    style: {
      margin: '28px 0',
      padding: '4px 0 4px 20px',
      borderLeft: '2px solid var(--ink-900)',
      fontFamily: 'var(--font-heading)',
      fontWeight: 700,
      fontSize: 22,
      lineHeight: '32px',
      fontStyle: 'italic',
      color: 'var(--ink-800)'
    }
  }, "Papers that do not follow the anonymisation rules will be desk-rejected without review.")), /*#__PURE__*/React.createElement("aside", {
    style: {
      position: 'sticky',
      top: 140,
      alignSelf: 'start',
      display: 'flex',
      flexDirection: 'column',
      gap: 16
    }
  }, /*#__PURE__*/React.createElement(Card, {
    variant: "inverse"
  }, /*#__PURE__*/React.createElement(Overline, {
    color: "var(--ink-300)"
  }, "Next deadline"), /*#__PURE__*/React.createElement("div", {
    style: {
      fontFamily: 'var(--font-heading)',
      fontWeight: 700,
      fontSize: 30,
      lineHeight: '36px',
      margin: '8px 0 4px'
    }
  }, "15 December 2025"), /*#__PURE__*/React.createElement(Tooltip, {
    content: "India Standard Time (UTC+5:30)"
  }, /*#__PURE__*/React.createElement("span", {
    style: {
      fontFamily: 'var(--font-mono)',
      fontSize: 13,
      color: 'var(--ink-300)',
      borderBottom: '1px dotted var(--ink-400)'
    }
  }, "23:59 IST")), /*#__PURE__*/React.createElement("div", {
    style: {
      marginTop: 20
    }
  }, /*#__PURE__*/React.createElement(Button, {
    variant: "accent",
    fullWidth: true,
    iconRight: "arrow-right",
    onClick: () => go('submit')
  }, "Open submission portal"))), /*#__PURE__*/React.createElement(Card, null, /*#__PURE__*/React.createElement(Overline, null, "Templates"), /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      flexDirection: 'column',
      gap: 8,
      marginTop: 12
    }
  }, /*#__PURE__*/React.createElement(Button, {
    variant: "secondary",
    size: "sm",
    iconLeft: "download"
  }, "LaTeX template"), /*#__PURE__*/React.createElement(Button, {
    variant: "secondary",
    size: "sm",
    iconLeft: "download"
  }, "Word template")))));
}
window.CallForPapersScreen = CallForPapersScreen;
})(); } catch (e) { __ds_ns.__errors.push({ path: "ui_kits/website/CallForPapersScreen.jsx", error: String((e && e.message) || e) }); }

// ui_kits/website/Chrome.jsx
try { (() => {
const {
  Button,
  Icon
} = window.STELMARDesignSystem_89726f;
function LogoLockup({
  height = 76
}) {
  return /*#__PURE__*/React.createElement("img", {
    src: window.SITE.logo,
    alt: "STELMAR 2026 \xB7 BML Munjal University",
    style: {
      height,
      width: 'auto',
      display: 'block'
    }
  });
}
function SiteNav({
  page,
  go
}) {
  const {
    SiteHeader
  } = window.STELMARDesignSystem_89726f;
  return /*#__PURE__*/React.createElement(SiteHeader, {
    logo: /*#__PURE__*/React.createElement(LogoLockup, {
      height: 64
    }),
    links: window.SITE.nav,
    active: page,
    onNavigate: v => go(v || 'home')
  });
}
function SiteFooter({
  go
}) {
  const col = (h, items) => /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      flexDirection: 'column',
      gap: 8
    }
  }, /*#__PURE__*/React.createElement("div", {
    style: {
      fontSize: 13,
      fontWeight: 600,
      letterSpacing: '0.06em',
      textTransform: 'uppercase',
      color: 'var(--ink-300)',
      marginBottom: 4
    }
  }, h), items.map(i => /*#__PURE__*/React.createElement("a", {
    key: i,
    style: {
      color: 'var(--paper-0)',
      textDecoration: 'none',
      fontSize: 15,
      cursor: 'pointer'
    }
  }, i)));
  return /*#__PURE__*/React.createElement("footer", {
    style: {
      background: 'var(--surface-header)',
      color: 'var(--paper-0)',
      marginTop: 96
    }
  }, /*#__PURE__*/React.createElement("div", {
    style: {
      maxWidth: 1200,
      margin: '0 auto',
      padding: '56px 32px 32px',
      display: 'grid',
      gridTemplateColumns: '2fr 1fr 1fr 1fr',
      gap: 32
    }
  }, /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      flexDirection: 'column',
      gap: 16
    }
  }, /*#__PURE__*/React.createElement(LogoLockup, {
    height: 64
  }), /*#__PURE__*/React.createElement("p", {
    style: {
      margin: 0,
      color: 'var(--ink-300)',
      fontSize: 14,
      lineHeight: '22px',
      maxWidth: 320
    }
  }, "Scholars' Conference on Sustainable Trends in Engineering, Law, Management, and Liberal Arts Research.")), col('Authors', ['Guidelines', 'Call for Papers', 'Portal', 'Timeline']), col('Attend', ['Speakers', 'Registrations', 'Sponsorship']), col('Conference', ['Committee', 'Contact'])), /*#__PURE__*/React.createElement("div", {
    style: {
      maxWidth: 1200,
      margin: '0 auto',
      padding: '16px 32px 24px',
      borderTop: '1px solid var(--ink-600)',
      fontSize: 13,
      color: 'var(--ink-400)'
    }
  }, "\xA9 2026 STELMAR \xB7 BML Munjal University"));
}
function Overline({
  children,
  color
}) {
  return /*#__PURE__*/React.createElement("div", {
    style: {
      fontFamily: 'var(--font-ui)',
      fontSize: 12,
      fontWeight: 600,
      letterSpacing: '0.08em',
      textTransform: 'uppercase',
      color: color || 'var(--ink-600)'
    }
  }, children);
}
function SectionHead({
  over,
  title,
  aside
}) {
  return /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      justifyContent: 'space-between',
      alignItems: 'flex-end',
      gap: 24,
      marginBottom: 28
    }
  }, /*#__PURE__*/React.createElement("div", null, /*#__PURE__*/React.createElement(Overline, null, over), /*#__PURE__*/React.createElement("h2", {
    style: {
      fontFamily: 'var(--font-heading)',
      fontWeight: 700,
      fontSize: 32,
      lineHeight: '40px',
      margin: '6px 0 0',
      color: 'var(--ink-900)'
    }
  }, title)), aside);
}
Object.assign(window, {
  LogoLockup,
  SiteNav,
  SiteFooter,
  Overline,
  SectionHead
});
})(); } catch (e) { __ds_ns.__errors.push({ path: "ui_kits/website/Chrome.jsx", error: String((e && e.message) || e) }); }

// ui_kits/website/HomeScreen.jsx
try { (() => {
function HomeScreen({
  go
}) {
  const {
    Card,
    Icon,
    Hero,
    GlassPill,
    Countdown,
    Button
  } = window.STELMARDesignSystem_89726f;
  const S = window.SITE;
  return /*#__PURE__*/React.createElement("main", null, /*#__PURE__*/React.createElement(Hero, {
    image: S.heroImage,
    title: S.title,
    minHeight: 720
  }, /*#__PURE__*/React.createElement(GlassPill, {
    size: "lg"
  }, S.theme), /*#__PURE__*/React.createElement(GlassPill, {
    icon: "calendar"
  }, S.dates, " \xB7 ", S.host), /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      gap: 16,
      marginTop: 16,
      flexWrap: 'wrap',
      justifyContent: 'center'
    }
  }, /*#__PURE__*/React.createElement(Button, {
    size: "lg",
    variant: "light",
    onClick: () => go('about')
  }, "About"), /*#__PURE__*/React.createElement(Button, {
    size: "lg",
    onClick: () => go('portal')
  }, "Submit Paper")), /*#__PURE__*/React.createElement(Countdown, {
    to: S.startsAt
  })), /*#__PURE__*/React.createElement("section", {
    id: "about",
    style: {
      maxWidth: 1200,
      margin: '0 auto',
      padding: '80px 32px 0'
    }
  }, /*#__PURE__*/React.createElement(SectionHead, {
    over: "Conference tracks",
    title: "Tracks"
  }), /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'grid',
      gridTemplateColumns: 'repeat(auto-fit, minmax(220px, 1fr))',
      gap: 20
    }
  }, S.tracks.map(([t, ic]) => /*#__PURE__*/React.createElement(Card, {
    key: t,
    style: {
      display: 'flex',
      flexDirection: 'column',
      gap: 14
    }
  }, /*#__PURE__*/React.createElement(Icon, {
    name: ic,
    size: 24
  }), /*#__PURE__*/React.createElement("div", {
    style: {
      fontWeight: 700,
      fontSize: 20,
      lineHeight: '26px'
    }
  }, t))))));
}
window.HomeScreen = HomeScreen;
})(); } catch (e) { __ds_ns.__errors.push({ path: "ui_kits/website/HomeScreen.jsx", error: String((e && e.message) || e) }); }

// ui_kits/website/ProgramScreen.jsx
try { (() => {
function ProgramScreen() {
  const {
    Tabs,
    SessionItem,
    Tag,
    Switch,
    Input,
    Toast
  } = window.STELMARDesignSystem_89726f;
  const DAYS = {
    d1: [['08:30', '09:15', 'Break', 'Registration & coffee', '', 'Foyer'], ['09:30', '10:30', 'Keynote', 'Reading at scale: what proceedings owe their readers', 'Prof. Helena Lindqvist', 'Main Hall'], ['11:00', '12:30', 'Paper session', '1A · Foundations of evaluation', 'Chair: R. Mendes', 'Room A'], ['11:00', '12:30', 'Paper session', '1B · Retrieval and indexing', 'Chair: S. Ivanova', 'Room B'], ['14:00', '15:30', 'Panel', 'Open peer review: what have we learned?', 'M. Tanaka, J. Ortiz, P. Nair', 'Main Hall'], ['16:00', '17:00', 'Award', 'Best paper session', '', 'Main Hall']],
    d2: [['09:30', '10:30', 'Keynote', 'Reproducibility as a first-class result', 'Dr. Kwame Asante', 'Main Hall'], ['11:00', '12:30', 'Paper session', '2A · Systems at scale', 'Chair: L. Brandt', 'Room A'], ['14:00', '15:30', 'Paper session', '2B · Data and benchmarks', 'Chair: A. Okafor', 'Room B']],
    d3: [['09:30', '10:30', 'Keynote', 'Open review, ten years on', 'Prof. Mei Tanaka', 'Main Hall'], ['11:00', '12:30', 'Paper session', '3A · Applications in health', 'Chair: T. Weber', 'Room A']],
    d4: [['09:00', '17:00', 'Workshop', 'Workshop on Scholarly Document Processing', 'Organisers: SDP committee', 'Room C']]
  };
  const [day, setDay] = React.useState('d1');
  const [saved, setSaved] = React.useState({
    '09:30Reading at scale: what proceedings owe their readers': true
  });
  const [mine, setMine] = React.useState(false);
  const [kind, setKind] = React.useState('All');
  const [toast, setToast] = React.useState(null);
  const rows = DAYS[day].filter(r => (kind === 'All' || r[2] === kind) && (!mine || saved[r[0] + r[3]]));
  const toggle = r => {
    const k = r[0] + r[3];
    const on = !saved[k];
    setSaved({
      ...saved,
      [k]: on
    });
    setToast(on ? 'Added to my programme' : 'Removed from my programme');
    setTimeout(() => setToast(null), 2500);
  };
  return /*#__PURE__*/React.createElement("main", {
    style: {
      maxWidth: 1200,
      margin: '0 auto',
      padding: '56px 32px 0'
    }
  }, /*#__PURE__*/React.createElement(Overline, {
    color: "var(--orange-600)"
  }, "BML Munjal University \xB7 29\u201331 January 2026"), /*#__PURE__*/React.createElement("h1", {
    style: {
      fontFamily: 'var(--font-heading)',
      fontWeight: 700,
      fontSize: 56,
      lineHeight: '60px',
      letterSpacing: '-0.01em',
      margin: '12px 0 28px'
    }
  }, "Programme"), /*#__PURE__*/React.createElement(Tabs, {
    size: "lg",
    value: day,
    onChange: setDay,
    items: [{
      value: 'd1',
      label: 'Day 1',
      meta: 'Thu 29 Jan'
    }, {
      value: 'd2',
      label: 'Day 2',
      meta: 'Fri 30 Jan'
    }, {
      value: 'd3',
      label: 'Day 3',
      meta: 'Sat 31 Jan'
    }]
  }), /*#__PURE__*/React.createElement("div", {
    style: {
      display: 'flex',
      alignItems: 'center',
      gap: 8,
      padding: '20px 0',
      flexWrap: 'wrap'
    }
  }, ['All', 'Keynote', 'Paper session', 'Panel', 'Workshop'].map(k => /*#__PURE__*/React.createElement(Tag, {
    key: k,
    selected: kind === k,
    onClick: () => setKind(k)
  }, k)), /*#__PURE__*/React.createElement("div", {
    style: {
      marginLeft: 'auto',
      display: 'flex',
      gap: 20,
      alignItems: 'center'
    }
  }, /*#__PURE__*/React.createElement(Switch, {
    checked: mine,
    onChange: setMine,
    label: "My programme only"
  }), /*#__PURE__*/React.createElement(Input, {
    iconLeft: "search",
    placeholder: "Search sessions",
    style: {
      width: 240
    }
  }))), /*#__PURE__*/React.createElement("div", {
    style: {
      borderTop: '1px solid var(--ink-900)'
    }
  }, rows.map(r => /*#__PURE__*/React.createElement(SessionItem, {
    key: r[0] + r[3],
    start: r[0],
    end: r[1],
    kind: r[2],
    title: r[3],
    speakers: r[4],
    room: r[5],
    saved: !!saved[r[0] + r[3]],
    onToggleSave: r[2] === 'Break' ? undefined : () => toggle(r)
  })), rows.length === 0 && /*#__PURE__*/React.createElement("div", {
    style: {
      padding: '40px 0',
      color: 'var(--ink-600)',
      fontSize: 15
    }
  }, "No sessions match. Save sessions with the bookmark icon to build your programme.")), toast && /*#__PURE__*/React.createElement("div", {
    style: {
      position: 'fixed',
      left: 24,
      bottom: 24,
      zIndex: 50
    }
  }, /*#__PURE__*/React.createElement(Toast, {
    tone: "success",
    title: toast
  })));
}
window.ProgramScreen = ProgramScreen;
})(); } catch (e) { __ds_ns.__errors.push({ path: "ui_kits/website/ProgramScreen.jsx", error: String((e && e.message) || e) }); }

// ui_kits/website/content.js
try { (() => {
// Sample content — edit freely. Screens read from window.SITE; nothing about layout is tied to it.
window.SITE = {
  logo: '../../assets/logo-lockup-header.png',
  shortName: 'STELMAR 2026',
  title: "2nd Scholars' Conference on Sustainable Trends in Engineering, Law, Management, and Liberal Arts Research 2026",
  theme: 'Sustainable Systems and Interdisciplinary Innovations for a Resilient Future',
  dates: '29–31 January 2026',
  startsAt: '2026-01-29T09:00:00+05:30',
  host: 'BML Munjal University',
  heroImage: null,
  nav: [{
    value: 'guidelines',
    label: 'Guidelines'
  }, {
    value: 'cfp',
    label: 'Call for Papers'
  }, {
    value: 'speakers',
    label: 'Speakers'
  }, {
    value: 'register',
    label: 'Registrations'
  }, {
    value: 'sponsorship',
    label: 'Sponsorship'
  }, {
    value: 'committee',
    label: 'Committee'
  }, {
    value: 'contact',
    label: 'Contact'
  }, {
    value: 'program',
    label: 'Timeline'
  }, {
    value: 'portal',
    label: 'Portal'
  }],
  tracks: [['Engineering', 'cpu'], ['Law', 'scale'], ['Management', 'briefcase'], ['Liberal Arts', 'book-open']]
};
})(); } catch (e) { __ds_ns.__errors.push({ path: "ui_kits/website/content.js", error: String((e && e.message) || e) }); }

__ds_ns.Badge = __ds_scope.Badge;

__ds_ns.Button = __ds_scope.Button;

__ds_ns.Icon = __ds_scope.Icon;

__ds_ns.IconButton = __ds_scope.IconButton;

__ds_ns.Tag = __ds_scope.Tag;

__ds_ns.Dialog = __ds_scope.Dialog;

__ds_ns.Toast = __ds_scope.Toast;

__ds_ns.Tooltip = __ds_scope.Tooltip;

__ds_ns.Checkbox = __ds_scope.Checkbox;

__ds_ns.Input = __ds_scope.Input;

__ds_ns.Radio = __ds_scope.Radio;

__ds_ns.Select = __ds_scope.Select;

__ds_ns.Switch = __ds_scope.Switch;

__ds_ns.Card = __ds_scope.Card;

__ds_ns.PaperCard = __ds_scope.PaperCard;

__ds_ns.SessionItem = __ds_scope.SessionItem;

__ds_ns.Tabs = __ds_scope.Tabs;

__ds_ns.Countdown = __ds_scope.Countdown;

__ds_ns.GlassPill = __ds_scope.GlassPill;

__ds_ns.Hero = __ds_scope.Hero;

__ds_ns.SiteHeader = __ds_scope.SiteHeader;

})();
