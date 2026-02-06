// pretty-bytes v7.1.0 — MIT License — Sindre Sorhus
// https://github.com/sindresorhus/pretty-bytes

const BYTE_UNITS = [
	'B',
	'kB',
	'MB',
	'GB',
	'TB',
	'PB',
	'EB',
	'ZB',
	'YB',
];

const BIBYTE_UNITS = [
	'B',
	'KiB',
	'MiB',
	'GiB',
	'TiB',
	'PiB',
	'EiB',
	'ZiB',
	'YiB',
];

const BIT_UNITS = [
	'b',
	'kbit',
	'Mbit',
	'Gbit',
	'Tbit',
	'Pbit',
	'Ebit',
	'Zbit',
	'Ybit',
];

const BIBIT_UNITS = [
	'b',
	'kibit',
	'Mibit',
	'Gibit',
	'Tibit',
	'Pibit',
	'Eibit',
	'Zibit',
	'Yibit',
];

const toLocaleString = (number, locale, options) => {
	let result = number;
	if (typeof locale === 'string' || Array.isArray(locale)) {
		result = number.toLocaleString(locale, options);
	} else if (locale === true || options !== undefined) {
		result = number.toLocaleString(undefined, options);
	}

	return result;
};

const log10 = numberOrBigInt => {
	if (typeof numberOrBigInt === 'number') {
		return Math.log10(numberOrBigInt);
	}

	const string = numberOrBigInt.toString(10);

	return string.length + Math.log10(`0.${string.slice(0, 15)}`);
};

const log = numberOrBigInt => {
	if (typeof numberOrBigInt === 'number') {
		return Math.log(numberOrBigInt);
	}

	return log10(numberOrBigInt) * Math.log(10);
};

const divide = (numberOrBigInt, divisor) => {
	if (typeof numberOrBigInt === 'number') {
		return numberOrBigInt / divisor;
	}

	const integerPart = numberOrBigInt / BigInt(divisor);
	const remainder = numberOrBigInt % BigInt(divisor);
	return Number(integerPart) + (Number(remainder) / divisor);
};

const applyFixedWidth = (result, fixedWidth) => {
	if (fixedWidth === undefined) {
		return result;
	}

	if (typeof fixedWidth !== 'number' || !Number.isSafeInteger(fixedWidth) || fixedWidth < 0) {
		throw new TypeError(`Expected fixedWidth to be a non-negative integer, got ${typeof fixedWidth}: ${fixedWidth}`);
	}

	if (fixedWidth === 0) {
		return result;
	}

	return result.length < fixedWidth ? result.padStart(fixedWidth, ' ') : result;
};

const buildLocaleOptions = options => {
	const {minimumFractionDigits, maximumFractionDigits} = options;

	if (minimumFractionDigits === undefined && maximumFractionDigits === undefined) {
		return undefined;
	}

	return {
		...(minimumFractionDigits !== undefined && {minimumFractionDigits}),
		...(maximumFractionDigits !== undefined && {maximumFractionDigits}),
		roundingMode: 'trunc',
	};
};

export default function prettyBytes(number, options) {
	if (typeof number !== 'bigint' && !Number.isFinite(number)) {
		throw new TypeError(`Expected a finite number, got ${typeof number}: ${number}`);
	}

	options = {
		bits: false,
		binary: false,
		space: true,
		nonBreakingSpace: false,
		...options,
	};

	const UNITS = options.bits
		? (options.binary ? BIBIT_UNITS : BIT_UNITS)
		: (options.binary ? BIBYTE_UNITS : BYTE_UNITS);

	const separator = options.space ? (options.nonBreakingSpace ? '\u00A0' : ' ') : '';

	const isZero = typeof number === 'number' ? number === 0 : number === 0n;
	if (options.signed && isZero) {
		const result = ` 0${separator}${UNITS[0]}`;
		return applyFixedWidth(result, options.fixedWidth);
	}

	const isNegative = number < 0;
	const prefix = isNegative ? '-' : (options.signed ? '+' : '');

	if (isNegative) {
		number = -number;
	}

	const localeOptions = buildLocaleOptions(options);
	let result;

	if (number < 1) {
		const numberString = toLocaleString(number, options.locale, localeOptions);
		result = prefix + numberString + separator + UNITS[0];
	} else {
		const exponent = Math.min(Math.floor(options.binary ? log(number) / Math.log(1024) : log10(number) / 3), UNITS.length - 1);
		number = divide(number, (options.binary ? 1024 : 1000) ** exponent);

		if (!localeOptions) {
			const minPrecision = Math.max(3, Math.floor(number).toString().length);
			number = number.toPrecision(minPrecision);
		}

		const numberString = toLocaleString(Number(number), options.locale, localeOptions);
		const unit = UNITS[exponent];
		result = prefix + numberString + separator + unit;
	}

	return applyFixedWidth(result, options.fixedWidth);
}
