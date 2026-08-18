/** Extract the value from an input/select/textarea change event. */
export function targetValue(e: Event): string {
	const eventTarget = e.currentTarget ?? e.target;
	if (
		eventTarget instanceof HTMLInputElement ||
		eventTarget instanceof HTMLSelectElement ||
		eventTarget instanceof HTMLTextAreaElement
	) {
		return eventTarget.value;
	}
	return "";
}

/** Extract the checked state from a checkbox change event. */
export function targetChecked(e: Event): boolean {
	const eventTarget = e.currentTarget ?? e.target;
	return eventTarget instanceof HTMLInputElement ? eventTarget.checked : false;
}

/** Get the typed target element from an event. */
export function target<T extends HTMLElement>(e: Event): T {
	return e.target as T;
}

/**
 * Typed event handler for input/select/textarea elements.
 * Usage: onInput={inputHandler((value) => setFoo(value))}
 */
export function inputHandler(fn: (value: string) => void): (e: Event) => void {
	return (e) => fn((e.target as HTMLInputElement).value);
}

/**
 * Typed event handler for checkbox elements.
 * Usage: onChange={checkHandler((checked) => setBar(checked))}
 */
export function checkHandler(fn: (checked: boolean) => void): (e: Event) => void {
	return (e) => fn((e.target as HTMLInputElement).checked);
}
