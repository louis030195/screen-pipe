import { $ } from 'bun'
import fs from 'fs/promises'
import os from 'os'
import path from 'path'

const isDevMode = process.env.SCREENPIPE_APP_DEV === 'true' || false;

const originalCWD = process.cwd()
// Change CWD to src-tauri
process.chdir(path.join(__dirname, '../src-tauri'))
const platform = {
	win32: 'windows',
	darwin: 'macos',
	linux: 'linux',
}[os.platform()]
const cwd = process.cwd()
console.log('cwd', cwd)
function hasFeature(name) {
	return process.argv.includes(`--${name}`) || process.argv.includes(name)
}

const config = {
	ffmpegRealname: 'ffmpeg',
	openblasRealname: 'openblas',
	clblastRealname: 'clblast',
	windows: {
		ffmpegName: 'ffmpeg-7.0-windows-desktop-vs2022-default',
		ffmpegUrl: 'https://unlimited.dl.sourceforge.net/project/avbuild/windows-desktop/ffmpeg-7.0-windows-desktop-vs2022-default.7z?viasf=1',

		openBlasName: 'OpenBLAS-0.3.26-x64',
		openBlasUrl: 'https://github.com/OpenMathLib/OpenBLAS/releases/download/v0.3.26/OpenBLAS-0.3.26-x64.zip',

		clblastName: 'CLBlast-1.6.2-windows-x64',
		clblastUrl: 'https://github.com/CNugteren/CLBlast/releases/download/1.6.2/CLBlast-1.6.2-windows-x64.zip',

		vcpkgPackages: ['opencl', 'onnxruntime-gpu'],
	},
	linux: {
		aptPackages: [
			'tesseract-ocr',
			'libtesseract-dev',
			'ffmpeg',
			'libopenblas-dev', // Runtime
			'pkg-config',
			'build-essential',
			'libglib2.0-dev',
			'libgtk-3-dev',
			'libwebkit2gtk-4.1-dev',
			'clang',
			'cmake', // Tauri
			'libavutil-dev',
			'libavformat-dev',
			'libavfilter-dev',
			'libavdevice-dev', // FFMPEG
			'libasound2-dev', // cpal
			'libomp-dev', // OpenMP in ggml.ai
			'libstdc++-12-dev', //ROCm
		],
	},
	macos: {
		ffmpegName: 'ffmpeg-7.0-macOS-default',
		ffmpegUrl: 'https://master.dl.sourceforge.net/project/avbuild/macOS/ffmpeg-7.0-macOS-default.tar.xz?viasf=1',
	},
}

async function findWget() {
	const possiblePaths = [
		'C:\\ProgramData\\chocolatey\\bin\\wget.exe',
		'C:\\Program Files\\Git\\mingw64\\bin\\wget.exe',
		'C:\\msys64\\usr\\bin\\wget.exe',
		'C:\\Windows\\System32\\wget.exe',
		'wget' // This will work if wget is in PATH
	];

	for (const wgetPath of possiblePaths) {
		try {
			await $`${wgetPath} --version`.quiet();
			console.log(`wget found at: ${wgetPath}`);
			return wgetPath;
		} catch (error) {
			// wget not found at this path, continue searching
		}
	}

	console.error('wget not found. Please install wget and make sure it\'s in your PATH.');
	process.exit(1);
}

// Export for Github actions
const exports = {
	ffmpeg: path.join(cwd, config.ffmpegRealname),
	openBlas: path.join(cwd, config.openblasRealname),
	clblast: path.join(cwd, config.clblastRealname, 'lib/cmake/CLBlast'),
	libClang: 'C:\\Program Files\\LLVM\\bin',
	cmake: 'C:\\Program Files\\CMake\\bin',
}

/* ########## Linux ########## */
if (platform == 'linux') {
	// Install APT packages
	await $`sudo apt-get update`
	if (hasFeature('opencl')) {
		config.linux.aptPackages.push('libclblast-dev')
	}
	for (const name of config.linux.aptPackages) {
		await $`sudo apt-get install -y ${name}`
	}

	// Copy screenpipe binary
	console.log('Copying screenpipe binary for Linux...');
	const potentialPaths = [
		path.join(__dirname, '..', '..', '..', '..', 'target', 'release', 'screenpipe'),
		path.join(__dirname, '..', '..', '..', '..', 'target', 'x86_64-unknown-linux-gnu', 'release', 'screenpipe'),
		path.join(__dirname, '..', '..', '..', 'target', 'release', 'screenpipe'),
		path.join(__dirname, '..', '..', 'target', 'release', 'screenpipe'),
		'/home/runner/work/screenpipe/screenpipe/target/release/screenpipe',
	];

	let copied = false;
	for (const screenpipeSrc of potentialPaths) {
		const screenpipeDest = path.join(cwd, 'screenpipe-x86_64-unknown-linux-gnu');
		try {
			await fs.copyFile(screenpipeSrc, screenpipeDest);
			console.log(`screenpipe binary copied successfully from ${screenpipeSrc}`);
			copied = true;
			break;
		} catch (error) {
			console.warn(`failed to copy screenpipe binary from ${screenpipeSrc}:`, error);
		}
	}

	if (!copied) {
		console.error("failed to copy screenpipe binary from any potential path.");
		// uncomment the following line if you want the script to exit on failure
		// process.exit(1);
	}


}

/* ########## Windows ########## */
if (platform == 'windows') {
	const wgetPath = await findWget();

	console.log('Copying screenpipe binary...');

	const potentialPaths = [
		path.join(__dirname, '..', '..', 'target', 'release', 'screenpipe.exe'),
		path.join(__dirname, '..', '..', 'target', 'x86_64-pc-windows-msvc', 'release', 'screenpipe.exe'),
		path.join(__dirname, '..', 'target', 'release', 'screenpipe.exe'),
		path.join(__dirname, '..', '..', 'target', 'release', 'screenpipe.exe'),
		'D:\\a\\screenpipe\\screenpipe\\target\\release\\screenpipe.exe',
	];

	let copied = false;
	for (const screenpipeSrc of potentialPaths) {
		const screenpipeDest = path.join(cwd, 'screenpipe-x86_64-pc-windows-msvc.exe');
		try {
			await fs.copyFile(screenpipeSrc, screenpipeDest);
			console.log(`Screenpipe binary copied successfully from ${screenpipeSrc}`);
			copied = true;
			break;
		} catch (error) {
			console.warn(`Failed to copy screenpipe binary from ${screenpipeSrc}:`, error);
		}
	}

	if (!copied) {
		console.error("Failed to copy screenpipe binary from any potential path.");
		// Uncomment the following line if you want the script to exit on failure
		// process.exit(1);
	}


	// Setup FFMPEG
	if (!(await fs.exists(config.ffmpegRealname))) {
		await $`${wgetPath} --tries=5 --retry-connrefused --waitretry=10 --secure-protocol=auto --no-check-certificate --show-progress ${config.windows.ffmpegUrl} -O ${config.windows.ffmpegName}.7z`
		await $`'C:\\Program Files\\7-Zip\\7z.exe' x ${config.windows.ffmpegName}.7z`
		await $`mv ${config.windows.ffmpegName} ${config.ffmpegRealname}`
		await $`rm -rf ${config.windows.ffmpegName}.7z`
		await $`mv ${config.ffmpegRealname}/lib/x64/* ${config.ffmpegRealname}/lib/`
	}

	// Setup Tesseract
	const tesseractName = 'tesseract-setup'
	const tesseractUrl = 'https://github.com/UB-Mannheim/tesseract/releases/download/v5.4.0.20240606/tesseract-ocr-w64-setup-5.4.0.20240606.exe'
	const tesseractInstaller = `${tesseractName}.exe`

	if (!(await fs.exists('tesseract'))) {
		console.log('Setting up Tesseract for Windows...')
		await $`${wgetPath} -nc  --no-check-certificate --show-progress ${tesseractUrl} -O ${tesseractInstaller}`
		await $`"${process.cwd()}\\${tesseractInstaller}" /S /D=C:\\Program Files\\Tesseract-OCR`
		await $`rm ${tesseractInstaller}`
		// Replace the mv command with xcopy
		await $`xcopy "C:\\Program Files\\Tesseract-OCR" tesseract /E /I /H /Y`
		// Optionally, remove the original directory if needed
		// await $`rmdir "C:\\Program Files\\Tesseract-OCR" /S /Q`
		console.log('Tesseract for Windows set up successfully.')
	} else {
		console.log('Tesseract for Windows already exists.')
	}

	// Add Tesseract to PATH
	process.env.PATH = `${process.cwd()}\\tesseract;${process.env.PATH}`

	// Setup ONNX Runtime
	const onnxRuntimeName = "onnxruntime-win-x64-gpu-1.19.2";
	const onnxRuntimeLibs = `${onnxRuntimeName}.zip`;
	const onnxRuntimeUrl = `https://github.com/microsoft/onnxruntime/releases/download/v1.19.2/${onnxRuntimeLibs}`
	if (!(await fs.exists(onnxRuntimeName))) {
		console.log('Setting up ONNX Runtime libraries for Windows...')
		try {
			await $`${wgetPath} -nc --no-check-certificate --show-progress ${onnxRuntimeUrl} -O ${onnxRuntimeLibs}`
			await $`unzip ${onnxRuntimeLibs} || tar -xf ${onnxRuntimeLibs} || echo "Done extracting"`;
			await $`rm -rf ${onnxRuntimeLibs} || rm ${onnxRuntimeLibs} -Recurse -Force || echo "Done cleaning up zip"`;
			console.log('ONNX Runtime libraries for Windows set up successfully.')
		} catch (error) {
			console.error('Error downloading or extracting ONNX Runtime:', error);
			console.log('Attempting alternative download method...');
			// Add alternative download method here
		}
	} else {
		console.log('ONNX Runtime libraries for Windows already exists.')
	}

	// Setup OpenBlas
	if (!(await fs.exists(config.openblasRealname)) && hasFeature('openblas')) {
		await $`${wgetPath} -nc --show-progress ${config.windows.openBlasUrl} -O ${config.windows.openBlasName}.zip`
		await $`"C:\\Program Files\\7-Zip\\7z.exe" x ${config.windows.openBlasName}.zip -o${config.openblasRealname}`
		await $`rm ${config.windows.openBlasName}.zip`
		fs.cp(path.join(config.openblasRealname, 'include'), path.join(config.openblasRealname, 'lib'), { recursive: true, force: true })
		// It tries to link only openblas.lib but our is libopenblas.lib`
		fs.cp(path.join(config.openblasRealname, 'lib/libopenblas.lib'), path.join(config.openblasRealname, 'lib/openblas.lib'))
	}

	// Setup CLBlast
	if (!(await fs.exists(config.clblastRealname)) && !hasFeature('cuda')) {
		await $`${wgetPath} -nc --show-progress ${config.windows.clblastUrl} -O ${config.windows.clblastName}.zip`
		await $`"C:\\Program Files\\7-Zip\\7z.exe" x ${config.windows.clblastName}.zip` // 7z file inside
		await $`"C:\\Program Files\\7-Zip\\7z.exe" x ${config.windows.clblastName}.7z` // Inner folder
		await $`mv ${config.windows.clblastName} ${config.clblastRealname}`
		await $`rm ${config.windows.clblastName}.zip`
		await $`rm ${config.windows.clblastName}.7z`
	}

	// Setup vcpkg packages with environment variables set inline
	await $`SystemDrive=${process.env.SYSTEMDRIVE} SystemRoot=${process.env.SYSTEMROOT} windir=${process.env.WINDIR} C:\\vcpkg\\vcpkg.exe install ${config.windows.vcpkgPackages}`.quiet()
}

async function getMostRecentBinaryPath(targetArch, paths) {
	const validPaths = await Promise.all(paths.map(async (path) => {
		if (await fs.exists(path)) {
			const { stdout } = await $`file ${path}`.quiet();
			const binaryArch = stdout.includes('arm64') ? 'arm64' :
				stdout.includes('x86_64') ? 'x86_64' : null;
			if (binaryArch === targetArch) {
				const stat = await fs.stat(path);
				return { path, mtime: stat.mtime };
			}
		}
		return null;
	}));

	const filteredPaths = validPaths.filter(Boolean);

	if (filteredPaths.length === 0) {
		return null;
	}

	return filteredPaths.reduce((mostRecent, current) =>
		current.mtime > mostRecent.mtime ? current : mostRecent
	).path;
}
/* ########## macOS ########## */
if (platform == 'macos') {

	const architectures = ['arm64', 'x86_64'];

	for (const arch of architectures) {
		if (process.env['SKIP_SCREENPIPE_SETUP']) {
			break;
		}
		console.log(`Setting up screenpipe bin for ${arch}...`);

		if (arch === 'arm64') {
			const paths = [
				"../../target/aarch64-apple-darwin/release/screenpipe",
				"../../target/release/screenpipe"
			];

			const mostRecentPath = await getMostRecentBinaryPath('arm64', paths);
			if (mostRecentPath) {
				await $`cp ${mostRecentPath} screenpipe-aarch64-apple-darwin`;
				console.log(`Copied most recent arm64 screenpipe binary from ${mostRecentPath}`);
			} else {
				console.error("No suitable arm64 screenpipe binary found");
			}
			// if the binary exists, hard code the fucking dylib
			if (await fs.exists('screenpipe-aarch64-apple-darwin') && !isDevMode) {
				await $`install_name_tool -change screenpipe-vision/lib/libscreenpipe_arm64.dylib @rpath/../Frameworks/libscreenpipe_arm64.dylib ./screenpipe-aarch64-apple-darwin`
				await $`install_name_tool -change screenpipe-vision/lib/libscreenpipe.dylib @rpath/../Frameworks/libscreenpipe.dylib ./screenpipe-aarch64-apple-darwin`
				console.log(`hard coded the FUCKING dylib`);
			} else if (await fs.exists('screenpipe-aarch64-apple-darwin') && isDevMode) {
				await $`install_name_tool -change screenpipe-vision/lib/libscreenpipe_arm64.dylib @executable_path/../Frameworks/libscreenpipe_arm64.dylib ./screenpipe-aarch64-apple-darwin`
				await $`install_name_tool -change screenpipe-vision/lib/libscreenpipe.dylib @executable_path/../Frameworks/libscreenpipe.dylib ./screenpipe-aarch64-apple-darwin`
				await $`install_name_tool -add_rpath @executable_path/../Frameworks ./screenpipe-aarch64-apple-darwin`
				console.log(`Updated dylib paths for arm64 in dev mode`);
			}


		} else if (arch === 'x86_64') {
			// copy screenpipe binary (more recent one)
			const paths = [
				"../../target/x86_64-apple-darwin/release/screenpipe",
				"../../target/release/screenpipe"
			];

			const mostRecentPath = await getMostRecentBinaryPath('x86_64', paths);

			if (mostRecentPath) {
				await $`cp ${mostRecentPath} screenpipe-x86_64-apple-darwin`;
				console.log(`Copied most recent x86_64 screenpipe binary from ${mostRecentPath}`);
			} else {
				console.error("No suitable x86_64 screenpipe binary found");
			}
			// hard code the fucking dylib
			if (await fs.exists('screenpipe-x86_64-apple-darwin') && !isDevMode) {
				await $`install_name_tool -change screenpipe-vision/lib/libscreenpipe_x86_64.dylib @rpath/../Frameworks/libscreenpipe_x86_64.dylib ./screenpipe-x86_64-apple-darwin`
				await $`install_name_tool -change screenpipe-vision/lib/libscreenpipe.dylib @rpath/../Frameworks/libscreenpipe.dylib ./screenpipe-x86_64-apple-darwin`
				console.log(`hard coded the FUCKING dylib`);
			}

		}

		console.log(`screenpipe for ${arch} set up successfully.`);
	}


	// Setup FFMPEG
	if (!(await fs.exists(config.ffmpegRealname))) {
		await $`wget -nc ${config.macos.ffmpegUrl} -O ${config.macos.ffmpegName}.tar.xz`
		await $`tar xf ${config.macos.ffmpegName}.tar.xz`
		await $`mv ${config.macos.ffmpegName} ${config.ffmpegRealname}`
		await $`rm ${config.macos.ffmpegName}.tar.xz`
	} else {
		console.log('FFMPEG already exists');
	}

	// Move and rename ffmpeg and ffprobe binaries
	const ffmpegSrc = path.join(cwd, config.ffmpegRealname, 'bin', 'ffmpeg');

	// For x86_64
	await fs.copyFile(ffmpegSrc, path.join(cwd, 'ffmpeg-x86_64-apple-darwin'));

	// For arm64
	await fs.copyFile(ffmpegSrc, path.join(cwd, 'ffmpeg-aarch64-apple-darwin'));

	console.log('Moved and renamed ffmpeg binary for externalBin');
}

// Nvidia
let cudaPath
if (hasFeature('cuda')) {
	if (process.env['CUDA_PATH']) {
		cudaPath = process.env['CUDA_PATH']
	} else if (platform === 'windows') {
		const cudaRoot = 'C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\'
		cudaPath = 'C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA\\v12.5'
		if (await fs.exists(cudaRoot)) {
			const folders = await fs.readdir(cudaRoot)
			if (folders.length > 0) {
				cudaPath = cudaPath.replace('v12.5', folders[0])
			}
		}
	}

	if (process.env.GITHUB_ENV) {
		console.log('CUDA_PATH', cudaPath)
	}

	if (platform === 'windows') {
		const windowsConfig = {
			bundle: {
				resources: {
					'ffmpeg\\bin\\x64\\*': './',
					'openblas\\bin\\*.dll': './',
					[`${cudaPath}\\bin\\cudart64_*`]: './',
					[`${cudaPath}\\bin\\cublas64_*`]: './',
					[`${cudaPath}\\bin\\cublasLt64_*`]: './',
					'tesseract\\*': './',
					'onnxruntime*\\lib\\*.dll': './',
				},
				externalBin: [
					'screenpipe'
				]
			},
		}
		await fs.writeFile('tauri.windows.conf.json', JSON.stringify(windowsConfig, null, 4))
	}
	if (platform === 'linux') {
		// Add cuda toolkit depends package
		const tauriConfigContent = await fs.readFile('tauri.linux.conf.json', { encoding: 'utf-8' })
		const tauriConfig = JSON.parse(tauriConfigContent)
		tauriConfig.bundle.linux.deb.depends.push('nvidia-cuda-toolkit')
		await fs.writeFile('tauri.linux.conf.json', JSON.stringify(tauriConfig, null, 4))
	}
}

if (hasFeature('opencl')) {
	if (platform === 'windows') {
		const tauriConfigContent = await fs.readFile('tauri.windows.conf.json', { encoding: 'utf-8' })
		const tauriConfig = JSON.parse(tauriConfigContent)
		tauriConfig.bundle.resources['clblast\\bin\\*.dll'] = './'
		tauriConfig.bundle.resources['C:\\vcpkg\\packages\\opencl_x64-windows\\bin\\*.dll'] = './'
		await fs.writeFile('tauri.windows.conf.json', JSON.stringify(tauriConfig, null, 4))
	}
}

// OpenBlas
if (hasFeature('openblas')) {
	if (platform === 'windows') {
		const tauriConfigContent = await fs.readFile('tauri.windows.conf.json', { encoding: 'utf-8' })
		const tauriConfig = JSON.parse(tauriConfigContent)
		tauriConfig.bundle.resources['openblas\\bin\\*.dll'] = './'
		await fs.writeFile('tauri.windows.conf.json', JSON.stringify(tauriConfig, null, 4))
	}
}

// ROCM
let rocmPath = '/opt/rocm'
if (hasFeature('rocm')) {
	if (process.env.GITHUB_ENV) {
		console.log('ROCM_PATH', rocmPath)
	}
	if (platform === 'linux') {
		// Add rocm toolkit depends package
		const tauriConfigContent = await fs.readFile('tauri.linux.conf.json', { encoding: 'utf-8' })
		const tauriConfig = JSON.parse(tauriConfigContent)
		tauriConfig.bundle.linux.deb.depends.push('rocm')
		await fs.writeFile('tauri.linux.conf.json', JSON.stringify(tauriConfig, null, 4))
	}
}

// Development hints
if (!process.env.GITHUB_ENV) {
	console.log('\nCommands to build 🔨:')
	// Get relative path to screenpipe-app-tauri folder
	const relativePath = path.relative(originalCWD, path.join(cwd, '..'))
	if (originalCWD != cwd && relativePath != '') {
		console.log(`cd ${relativePath}`)
	}
	console.log('bun install')
	if (platform == 'windows') {
		console.log(`$env:FFMPEG_DIR = "${exports.ffmpeg}"`)
		console.log(`$env:OPENBLAS_PATH = "${exports.openBlas}"`)
		console.log(`$env:LIBCLANG_PATH = "${exports.libClang}"`)
		console.log(`$env:PATH += "${exports.cmake}"`)
		if (hasFeature('older-cpu')) {
			console.log(`$env:WHISPER_NO_AVX = "ON"`)
			console.log(`$env:WHISPER_NO_AVX2 = "ON"`)
			console.log(`$env:WHISPER_NO_FMA = "ON"`)
			console.log(`$env:WHISPER_NO_F16C = "ON"`)
		}
		if (hasFeature('cuda')) {
			console.log(`$env:CUDA_PATH = "${cudaPath}"`)
		}
		if (hasFeature('opencl')) {
			console.log(`$env:CLBlast_DIR = "${exports.clblast}"`)
		}
		if (hasFeature('rocm')) {
			console.log(`$env:ROCM_VERSION = "6.1.2"`)
			console.log(`$env:ROCM_PATH = "${rocmPath}"`)
		}
	}
	if (!process.env.GITHUB_ENV) {
		console.log('bun tauri build')
	}
}

// Config Github ENV
if (process.env.GITHUB_ENV) {
	console.log('Adding ENV')
	if (platform == 'macos' || platform == 'windows') {
		const ffmpeg = `FFMPEG_DIR=${exports.ffmpeg}\n`
		console.log('Adding ENV', ffmpeg)
		await fs.appendFile(process.env.GITHUB_ENV, ffmpeg)
	}
	if (platform == 'macos') {
		const embed_metal = 'WHISPER_METAL_EMBED_LIBRARY=ON'
		await fs.appendFile(process.env.GITHUB_ENV, embed_metal)
	}
	if (platform == 'windows') {
		const openblas = `OPENBLAS_PATH=${exports.openBlas}\n`
		console.log('Adding ENV', openblas)
		await fs.appendFile(process.env.GITHUB_ENV, openblas)

		if (hasFeature('opencl')) {
			const clblast = `CLBlast_DIR=${exports.clblast}\n`
			console.log('Adding ENV', clblast)
			await fs.appendFile(process.env.GITHUB_ENV, clblast)
		}

		if (hasFeature('older-cpu')) {
			await fs.appendFile(process.env.GITHUB_ENV, `WHISPER_NO_AVX=ON\n`)
			await fs.appendFile(process.env.GITHUB_ENV, `WHISPER_NO_AVX2=ON\n`)
			await fs.appendFile(process.env.GITHUB_ENV, `WHISPER_NO_FMA=ON\n`)
			await fs.appendFile(process.env.GITHUB_ENV, `WHISPER_NO_F16C=ON\n`)
		}
	}
}

// --dev or --build
const action = process.argv?.[2]
if (action?.includes('--build' || action.includes('--dev'))) {
	process.chdir(path.join(cwd, '..'))
	process.env['FFMPEG_DIR'] = exports.ffmpeg
	if (platform === 'windows') {
		process.env['OPENBLAS_PATH'] = exports.openBlas
		process.env['CLBlast_DIR'] = exports.clblast
		process.env['LIBCLANG_PATH'] = exports.libClang
		process.env['PATH'] = `${process.env['PATH']};${exports.cmake}`
	}
	if (platform == 'macos') {
		process.env['WHISPER_METAL_EMBED_LIBRARY'] = 'ON'
	}
	await $`bun install`
	await $`bunx tauri ${action.includes('--dev') ? 'dev' : 'build'}`
}
