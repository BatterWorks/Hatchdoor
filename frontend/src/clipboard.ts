export async function copyText(value: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(value);
      return true;
    }
  } catch {
    // Fallback below for non-secure contexts / unsupported clipboard API.
  }

  const target = document.createElement("span");
  target.textContent = value;
  target.setAttribute("contenteditable", "true");
  target.style.position = "fixed";
  target.style.top = "-1000px";
  target.style.left = "-1000px";
  target.style.whiteSpace = "pre";
  document.body.appendChild(target);

  const selection = window.getSelection();
  const range = document.createRange();
  range.selectNodeContents(target);
  selection?.removeAllRanges();
  selection?.addRange(range);
  try {
    return document.execCommand("copy");
  } finally {
    selection?.removeAllRanges();
    document.body.removeChild(target);
  }
}
