/* Adapted from Tailor. Blue Oak Model License 1.0.0:
 * https://blueoakcouncil.org/license/1.0.0 */
// Copy buttons: <button data-copy="id"> copies the text of the element with that id.
document.querySelectorAll("button[data-copy]").forEach((button) => {
  const source = document.getElementById(button.dataset.copy);
  if (!source || !navigator.clipboard) return;
  const status = button.parentElement.querySelector('[role="status"]');
  let timer;

  button.hidden = false;
  button.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(source.textContent.trim());
    } catch {
      if (status) status.textContent = "Copy failed. Select and copy the command.";
      return;
    }
    button.toggleAttribute("data-copied", true);
    if (status) status.textContent = "Copied";
    clearTimeout(timer);
    timer = setTimeout(() => {
      button.removeAttribute("data-copied");
      if (status) status.textContent = "";
    }, 2000);
  });
});
