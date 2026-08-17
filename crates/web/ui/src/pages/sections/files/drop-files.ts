export interface UploadFile {
	file: File;
	relativePath: string;
}

export interface UploadDirectory {
	relativePath: string;
}

export interface DroppedContents {
	files: UploadFile[];
	directories: UploadDirectory[];
}

interface DroppedEntry {
	isFile: boolean;
	isDirectory: boolean;
	name: string;
}

interface DroppedFileEntry extends DroppedEntry {
	file: (success: (file: File) => void, error?: (error: DOMException) => void) => void;
}

interface DroppedDirectoryReader {
	readEntries: (success: (entries: DroppedEntry[]) => void, error?: (error: DOMException) => void) => void;
}

interface DroppedDirectoryEntry extends DroppedEntry {
	createReader: () => DroppedDirectoryReader;
}

function cleanRelativePath(path: string): string {
	return path
		.replaceAll("\\", "/")
		.split("/")
		.filter((part) => part && part !== ".")
		.join("/");
}

function addParentDirectories(path: string, directories: Set<string>): void {
	const parts = cleanRelativePath(path).split("/").filter(Boolean);
	parts.pop();
	for (let index = 1; index <= parts.length; index += 1) {
		directories.add(parts.slice(0, index).join("/"));
	}
}

export function contentsFromFileList(fileList: FileList | File[]): DroppedContents {
	const files = Array.from(fileList).map((file) => {
		const relativePath = cleanRelativePath(file.webkitRelativePath || file.name);
		return { file, relativePath };
	});
	const directories = new Set<string>();
	for (const item of files) addParentDirectories(item.relativePath, directories);
	return {
		files,
		directories: Array.from(directories, (relativePath) => ({ relativePath })),
	};
}

function readFile(entry: DroppedFileEntry): Promise<File> {
	return new Promise((resolve, reject) => entry.file(resolve, reject));
}

async function readAllEntries(entry: DroppedDirectoryEntry): Promise<DroppedEntry[]> {
	const reader = entry.createReader();
	const entries: DroppedEntry[] = [];
	for (;;) {
		const batch = await new Promise<DroppedEntry[]>((resolve, reject) => reader.readEntries(resolve, reject));
		if (batch.length === 0) return entries;
		entries.push(...batch);
	}
}

async function walkEntry(
	entry: DroppedEntry,
	parentPath: string,
	files: UploadFile[],
	directories: Set<string>,
): Promise<void> {
	const relativePath = cleanRelativePath(parentPath ? `${parentPath}/${entry.name}` : entry.name);
	if (entry.isFile) {
		const file = await readFile(entry as DroppedFileEntry);
		files.push({ file, relativePath });
		return;
	}
	if (!entry.isDirectory) return;

	directories.add(relativePath);
	const children = await readAllEntries(entry as DroppedDirectoryEntry);
	await Promise.all(children.map((child) => walkEntry(child, relativePath, files, directories)));
}

export async function collectDroppedContents(dataTransfer: DataTransfer): Promise<DroppedContents> {
	const entries: DroppedEntry[] = [];
	for (const item of Array.from(dataTransfer.items)) {
		const entryItem = item as unknown as { webkitGetAsEntry?: () => DroppedEntry | null };
		const entry = entryItem.webkitGetAsEntry?.() ?? null;
		if (entry) entries.push(entry);
	}

	if (entries.length === 0) return contentsFromFileList(dataTransfer.files);

	const files: UploadFile[] = [];
	const directories = new Set<string>();
	await Promise.all(entries.map((entry) => walkEntry(entry, "", files, directories)));
	return {
		files,
		directories: Array.from(directories, (relativePath) => ({ relativePath })),
	};
}
