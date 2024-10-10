const { core } = Deno;
const { ops } = core;
const Path = {
    isAbsolute: (path) => path.startsWith('/') || /^[A-Za-z]:\\/.test(path),
    join: (...parts) => parts.join('/').replace(/\/+/g, '/'),
};

function argsToMessage(...args) {
    return args.map((arg) => JSON.stringify(arg)).join(" ");
}

const sendLog = async (level, ...args) => {
    const message = argsToMessage(...args);
    const logApiUrl = process.env.SCREENPIPE_LOG_API_URL || "http://localhost:11435/log";
    const pipeId = globalThis.metadata.id || "unknown";

    try {
        await ops.op_fetch(logApiUrl, {
            method: "POST",
            headers: {
                "Content-Type": "application/json",
            },
            body: JSON.stringify({
                pipeId,
                level,
                message,
                timestamp: new Date().toISOString(),
            }),
        });
    } catch (error) {
        // core.print(`[js][error]: Failed to send log: ${error}\n`, true);
    }
};

const console = {
    log: (...args) => {
        core.print(`[pipe][${globalThis.metadata.id}][info]: ${argsToMessage(...args)}\n`, false);
        sendLog("info", ...args);
    },
    error: (...args) => {
        core.print(`[pipe][${globalThis.metadata.id}][error]: ${argsToMessage(...args)}\n`, true);
        sendLog("error", ...args);
    },
    warn: (...args) => {
        core.print(`[pipe][${globalThis.metadata.id}][warn]: ${argsToMessage(...args)}\n`, true);
        sendLog("warn", ...args);
    }
};

globalThis.console = console;

const pipe = {
    readFile: (path) => {
        let fullPath;
        if (Path.isAbsolute(path)) {
            fullPath = path;
        } else {
            const pipeDir = `${process.env.SCREENPIPE_DIR}/pipes/${globalThis.metadata.id}`;
            fullPath = Path.join(pipeDir, path);
        }
        return ops.op_read_file(fullPath);
    },

    removeFile: (path) => {
        let fullPath;
        if (Path.isAbsolute(path)) {
            fullPath = path;
        } else {
            const pipeDir = `${process.env.SCREENPIPE_DIR}/pipes/${globalThis.metadata.id}`;
            fullPath = Path.join(pipeDir, path);
        }
        return ops.op_remove_file(fullPath);
    },

    get: async (url) => {
        const response = await ops.op_fetch_get(url);
        return JSON.parse(response);
    },
    post: async (url, body) => {
        const response = await ops.op_fetch_post(url, body);
        return JSON.parse(response);
    },
    fetch: async (url, options) => {
        try {
            const responseString = await ops.op_fetch(url, options);
            const response = JSON.parse(responseString);
            return {
                ok: response.status >= 200 && response.status < 300,
                status: response.status,
                statusText: response.statusText,
                headers: response.headers, // Use the headers directly without wrapping in Headers object
                text: async () => response.text,
                json: async () => {
                    try {
                        return JSON.parse(response.text);
                    } catch (error) {
                        console.error("Error parsing JSON:", error);
                        return response.text;
                    }
                },
            };
        } catch (error) {
            console.error("Fetch error:", error);
            throw error;
        }
    },
    sendNotification: async ({ title, body }) => {
        // try to fetch this url, if not live do try to send the notification to the server
        const notificationApiUrl = process.env.SCREENPIPE_SERVER_URL || "http://localhost:11435";

        try {
            const response = await ops.op_fetch(notificationApiUrl + "/notify", {
                headers: {
                    "Content-Type": "application/json",
                },
                method: "POST",
                body: JSON.stringify({ title, body })
            });
            console.log("Notification sent:", response);
            return JSON.parse(response);
        } catch (error) {
            console.log("Failed to send notification to server, is your notification server running?");
            return false;
        }
    },
    loadConfig: async () => {
        try {
            console.log("Attempting to load pipe.json");
            const configContent = await ops.op_read_file(process.env.SCREENPIPE_DIR + "/pipes/" + globalThis.metadata.id + "/pipe.json");
            console.log("pipe.json content:", configContent);
            const parsedConfig = JSON.parse(configContent);
            console.log("Parsed config:", parsedConfig);

            // Process the fields and set the config values
            const config = {};
            parsedConfig.fields.forEach(field => {
                config[field.name] = field.value !== undefined ? field.value : field.default;
            });

            pipe.config = config; // Set the processed config
            console.log("Processed config:", config);
            return config;
        } catch (error) {
            console.error("Error loading pipe.json:", error);
            pipe.config = {}; // Set an empty object if loading fails
            return {};
        }
    },
    sendEmail: async ({ to, from, password, subject, body, contentType = 'text/plain' }) => {
        try {
            await ops.op_send_email(to, from, password, subject, body, contentType);
            return true;
        } catch (error) {
            console.error("Error sending email:", error);
            return false;
        }
    },
    // isEnabled: async () => {
    //     return ops.op_is_enabled();
    // }
};

function encodeQueryData(data) {
    return Object.keys(data)
        .map(key => `${encodeURIComponent(key)}=${encodeURIComponent(data[key])}`)
        .join('&');
}

pipe.queryScreenpipe = async (params) => {
    try {
        const queryParams = {
            content_type: params.content_type || 'all',
            limit: params.limit || 50,
            offset: params.offset || 0,
            start_time: params.start_time || new Date(Date.now() - 3600000).toISOString(),
            end_time: params.end_time || new Date().toISOString(),
            min_length: params.min_length || 50,
            max_length: params.max_length || 10000,
            include_frames: params.include_frames || false,
        };

        if (params.q) queryParams.q = params.q;
        if (params.app_name) queryParams.app_name = params.app_name;
        if (params.window_name) queryParams.window_name = params.window_name;

        const queryString = encodeQueryData(queryParams);
        const url = `http://localhost:3030/search?${queryString}`;
        console.log("calling screenpipe", url);

        const response = await pipe.fetch(url);
        if (!response.ok) {
            const text = await response.text();
            throw new Error(`http error! status: ${response.status} ${text}`);
        }
        return await response.json();
    } catch (error) {
        console.error("error querying screenpipe:", error);
        return null;
    }
};

const fs = {
    readFile: async (path) => {
        return await ops.op_read_file(path);
    },
    writeFile: async (path, contents) => {
        return await ops.op_write_file(path, contents);
    },
    readdir: async (path) => {
        return await ops.op_readdir(path);
    },
    mkdir: async (path) => {
        return await ops.op_create_dir(path);
    }
};

const path = {
    join: (...paths) => {
        const sep = process.env.OS === "windows" ? "\\" : "/";
        // This implementation works on both Unix and Windows
        return paths.join(sep).replace(new RegExp(`\\${sep}+`, 'g'), sep);
    },
};

globalThis.fs = fs;
globalThis.path = path;

globalThis.setTimeout = (callback, delay) => {
    ops.op_set_timeout(delay).then(callback);
};
globalThis.pipe = pipe;
globalThis.pipe.metadata = globalThis.metadata;
globalThis.fetch = pipe.fetch;
globalThis.loadConfig = pipe.loadConfig;


pipe.extractJsonFromLlmResponse = (response) => {
    // Remove any markdown code block syntax
    let cleaned = response.replace(/^```(?:json)?\s*|\s*```$/g, "");

    console.log("cleaned", cleaned);

    // Try to find JSON-like content
    const jsonMatch = cleaned.match(/\{[\s\S]*\}/);

    console.log("jsonMatch", jsonMatch);
    if (jsonMatch) {
        cleaned = jsonMatch[0];
    }

    console.log("cleaned", cleaned);
    // Remove any non-JSON content before or after the main object
    cleaned = cleaned.replace(/^[^{]*/, "").replace(/[^}]*$/, "");

    console.log("cleaned", cleaned);

    // Replace any escaped newlines and remove actual newlines
    cleaned = cleaned.replace(/\\n/g, "").replace(/\n/g, "");

    console.log("cleaned", cleaned);

    // Handle nested JSON strings
    cleaned = cleaned.replace(/"(\\"|[^"])*"/g, (match) => {
        return match.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
    });

    console.log("cleaned", cleaned);

    try {
        return JSON.parse(cleaned);
    } catch (error) {
        console.warn("failed to parse json:", error);
        console.warn("cleaned content:", cleaned);

        // Attempt to fix common issues
        cleaned = cleaned
            .replace(/,\s*}/g, "}") // Remove trailing commas
            .replace(/'/g, '"') // Replace single quotes with double quotes
            .replace(/(\w+):/g, '"$1":') // Add quotes to keys
            .replace(/:\s*'([^']*)'/g, ': "$1"') // Replace single-quoted values with double-quoted values
            .replace(/\\"/g, '"') // Replace escaped double quotes with regular double quotes
            .replace(/"{/g, '{"') // Fix improperly escaped nested JSON
            .replace(/}"/g, '"}');

        try {
            return JSON.parse(cleaned);
        } catch (secondError) {
            console.warn("failed to parse json after attempted fixes:", secondError);
            throw new Error("invalid json format in llm response");
        }
    }
};

