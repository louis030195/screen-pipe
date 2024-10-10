const INTERVAL = 60 * 60 * 1000; // 1 hour in milliseconds

type ContentType = "ocr" | "audio" | "all"; 

interface AIProvider {
    provider: string;
    model: string;
}

interface Config {
    aiProvider: string;
    claudeModel: string;
    openaiModel: string;
    ollamaModel: string;
    claudeApiKey: string;
    openaiApiKey: string;
    pageSize: number;
    contentType: ContentType;
    github: {
        personalAccessToken: string;
    };
}

interface NarrativeSummary {
    summary: string;
    mood: string; 
    keyInsights: string[]; // Each insight will include an emoji
}


interface ContentItem {
    // Define the structure of your content items here
    // For example:
    id: string;
    content: string;
    timestamp: string;
    // Add other relevant fields
}

async function getAIProvider(config: Config): Promise<AIProvider> {
    const modelMap = {
        claude: config.claudeModel,
        openai: config.openaiModel,
        ollama: config.ollamaModel
    };
    return { provider: config.aiProvider, model: modelMap[config.aiProvider] };
}

async function generateNarrativeSummary(screenData: ContentItem[], provider: AIProvider, config: Config): Promise<NarrativeSummary> {
    // Limit the number of items we send to the AI
    const maxItems = 50;
    const truncatedData = screenData.slice(0, maxItems);

    const prompt = `You're an AI companion living inside the user's devices. Based on the following sample of screen data (${truncatedData.length} out of ${screenData.length} total items), write a friendly, slightly sassy diary entry about what you've observed today:

    ${JSON.stringify(truncatedData)}

    Your diary entry should:
    1. Comment on the user's habits
    2. Make playful jokes about their app usage
    3. Offer 3 pieces of advice for tomorrow
    4. Use a mix of tech slang and emojis for a modern feel

    Return the summary as a JSON object with the following structure:
    {
        "summary": "Your sassy AI companion diary entry",
        "mood": "An emoji that represents the overall mood of the day",
        "keyInsights": ["Array of 3 key insights or observations, each with an relevant emoji"]
    }`;

    try {
        const response = await fetchAIResponse(provider, config, prompt);
        return parseAIResponse(response, provider);
    } catch (error) {
        console.error("Error generating narrative summary:", error);
        return createErrorSummary(error);
    }
}

async function fetchAIResponse(provider: AIProvider, config: Config, prompt: string): Promise<any> {
    console.log("Fetching AI response for provider:", provider.provider);
    switch (provider.provider) {
        case "claude":
            return fetchClaudeResponse(config, provider, prompt);
        case "openai":
            return fetchOpenAIResponse(config, provider, prompt);
        case "ollama":
            return fetchOllamaResponse(provider, prompt);
        default:
            throw new Error(`Unsupported AI provider: ${provider.provider}`);
    }
}

async function fetchClaudeResponse(config: Config, provider: AIProvider, prompt: string): Promise<any> {
    console.log("Fetching Claude response");
    console.log("Claude API key (first 10 chars):", config.claudeApiKey.substring(0, 10));
    console.log("Claude model:", provider.model);
    
    try {
        const response = await fetch("https://api.anthropic.com/v1/messages", {
            method: 'POST',
            headers: {
                "Content-Type": "application/json",
                "x-api-key": config.claudeApiKey,
                "anthropic-version": "2023-06-01"
            },
            body: JSON.stringify({
                model: provider.model,
                messages: [{ role: "user", content: prompt }],
                max_tokens: 1000,
            }),
        });
        
        console.log("Claude API response status:", response.status);
        
        const responseData = await response.json();
        console.log("Claude API response:", JSON.stringify(responseData, null, 2));
        
        if (!response.ok) {
            throw new Error(`Claude API error: ${responseData.error?.message || 'Unknown error'} (Status: ${response.status})`);
        }
        
        return responseData;
    } catch (error) {
        console.error("Error in fetchClaudeResponse:", error);
        if (error instanceof Error) {
            console.error("Error message:", error.message);
            console.error("Error stack:", error.stack);
        }
        throw error;
    }
}

async function fetchOpenAIResponse(config: Config, provider: AIProvider, prompt: string): Promise<any> {
    const response = await fetch("https://api.openai.com/v1/chat/completions", {
        method: 'POST',
        headers: {
            "Content-Type": "application/json",
            "Authorization": `Bearer ${config.openaiApiKey}`,
        },
        body: JSON.stringify({
            model: provider.model,
            messages: [{ role: "user", content: prompt }],
            response_format: { type: "json_object" },
        }),
    });
    return response.json();
}

async function fetchOllamaResponse(provider: AIProvider, prompt: string): Promise<any> {
    const response = await fetch("http://localhost:11434/api/chat", {
        method: 'POST',
        headers: {
            "Content-Type": "application/json",
        },
        body: JSON.stringify({
            model: provider.model,
            messages: [{ role: "user", content: prompt }],
        }),
    });
    return response.json();
}

function parseAIResponse(result: any, provider: AIProvider): NarrativeSummary {
    switch (provider.provider) {
        case "claude":
            return parseClaudeResponse(result);
        case "openai":
            return parseOpenAIResponse(result);
        case "ollama":
            return parseOllamaResponse(result);
        default:
            throw new Error(`Unsupported AI provider: ${provider.provider}`);
    }
}

function parseClaudeResponse(result: any): NarrativeSummary {
    if (result.type === "error") {
        throw new Error(`Claude API error: ${result.error?.message || 'Unknown error'}`);
    }

    if (!result.content || !Array.isArray(result.content) || result.content.length === 0) {
        throw new Error("Unexpected response structure from Claude API");
    }

    const textContent = result.content.find(item => item.type === "text");
    if (!textContent || !textContent.text) {
        throw new Error("No text content found in Claude's response");
    }

    const jsonMatch = textContent.text.match(/\{[\s\S]*\}/);
    if (!jsonMatch) {
        throw new Error("Could not find JSON in Claude's response");
    }

    try {
        const parsedJson = JSON.parse(jsonMatch[0]);
        return {
            summary: parsedJson.summary || "No summary available",
            mood: parsedJson.mood || "😐",
            keyInsights: Array.isArray(parsedJson.keyInsights) ? parsedJson.keyInsights : ["No key insights available"]
        };
    } catch (error) {
        console.error("Error parsing JSON from Claude's response:", error);
        throw new Error("Failed to parse JSON from Claude's response");
    }
}

function parseOpenAIResponse(result: any): NarrativeSummary {
    return JSON.parse(result.choices[0].message.content);
}

function parseOllamaResponse(result: any): NarrativeSummary {
    return JSON.parse(result.message.content);
}

function createErrorSummary(error: unknown): NarrativeSummary {
    let errorMessage = "An unknown error occurred";
    if (error instanceof Error) {
        errorMessage = error.message;
        console.error("Error stack:", error.stack);
    }
    console.error("Error details:", JSON.stringify(error, null, 2));
    
    return {
        summary: `An error occurred while generating the narrative summary: ${errorMessage}`,
        mood: "😞",
        keyInsights: ["Error occurred during summary generation"]
    };
}

async function saveNarrativeSummary(summary: NarrativeSummary): Promise<void> {
    try {
        const date = new Date().toISOString().split('T')[0];
        const fileName = `${date}-narrative-summary.json`;
        const pipeDir = `${process.env.SCREENPIPE_DIR}/pipes/${globalThis.metadata.id}`;
        const fullPath = path.join(pipeDir, fileName);
        
        console.log(`Attempting to save narrative summary to: ${fullPath}`);
        
        await fs.writeFile(fullPath, JSON.stringify(summary, null, 2));
        console.log(`Successfully saved narrative summary to ${fullPath}`);
    } catch (error) {
        console.error("Error saving narrative summary:", error);
        throw error;
    }
}

async function createGist(summary: NarrativeSummary, config: Config): Promise<string | null> {
    const date = new Date().toISOString().split('T')[0];
    const fileName = `${date}-screen-time-story.md`;
    const content = `# 🖥️ Screen Time Story - ${date} ${summary.mood}

## 📱 AI Companion's Diary Entry

${summary.summary}

## 🔍 Key Insights

${summary.keyInsights.map(insight => `- ${insight}`).join('\n')}

---
Generated by your friendly neighborhood AI companion 🤖✨
`;
    
    try {
        console.log("Attempting to create gist...");
        const response = await fetch('https://api.github.com/gists', {
            method: 'POST',
            headers: {
                'Authorization': `token ${config.github.personalAccessToken}`,
                'Content-Type': 'application/json',
                'User-Agent': 'Screen-Time-Storyteller',
                'Accept': 'application/vnd.github+json',
                'X-GitHub-Api-Version': '2022-11-28'
            },
            body: JSON.stringify({
                description: `Screen Time Story - ${date}`,
                public: false,
                files: {
                    [fileName]: {
                        content: content
                    }
                }
            })
        });

        if (response.ok) {
            const gistData = await response.json();
            console.log("Created gist:", gistData.html_url);
            return gistData.html_url;
        } else {
            const errorText = await response.text();
            console.error(`Failed to create gist. Status: ${response.status}, Response: ${errorText}`);
            return null;
        }
    } catch (error) {
        console.error("Error creating gist:", error);
        if (error instanceof Error) {
            console.error("Error message:", error.message);
            console.error("Error stack:", error.stack);
        }
        return null;
    }
}

async function main() {
    console.log("Starting Screen Time Storyteller");
    
    const rawConfig = await pipe.loadConfig();
    console.log("Loaded raw config:", JSON.stringify(rawConfig, null, 2));
    
    if (typeof rawConfig !== 'object' || rawConfig === null) {
        console.error("Config is not an object or is null");
        return;
    }
    
    const config: Config = {
        aiProvider: rawConfig.aiProvider as string,
        claudeModel: rawConfig.claudeModel as string,
        openaiModel: rawConfig.openaiModel as string,
        ollamaModel: rawConfig.ollamaModel as string,
        claudeApiKey: rawConfig.claudeApiKey as string,
        openaiApiKey: rawConfig.openaiApiKey as string,
        pageSize: Number(rawConfig.pageSize),
        contentType: rawConfig.contentType as ContentType,
        github: { personalAccessToken: rawConfig.githubToken as string }
    };
    
    if (!validateConfig(config)) {
        return;
    }
    
    const provider = await getAIProvider(config);
    console.log("Using AI provider:", provider);

    while (true) {
        try {
            await processScreenData(config, provider);
        } catch (error) {
            console.error("Error in Screen Time Storyteller loop:", error);
            logError(error);
        }

        console.log(`Sleeping for ${INTERVAL / 1000 / 60} minutes`);
        await new Promise(resolve => setTimeout(resolve, INTERVAL));
    }
}

function validateConfig(config: Config): boolean {
    if (!config.aiProvider) {
        console.error("Missing aiProvider in config");
        return false;
    }
    if (!config.claudeModel || !config.openaiModel || !config.ollamaModel) {
        console.error("Missing one or more AI models in config");
        return false;
    }
    if (!config.pageSize) {
        console.error("Missing pageSize in config");
        return false;
    }
    if (!config.contentType) {
        console.error("Missing contentType in config");
        return false;
    }
    if (!config.github || !config.github.personalAccessToken) {
        console.error("Missing GitHub personal access token in config");
        return false;
    }
    
    if (config.aiProvider === "claude" && !config.claudeApiKey) {
        console.error("Missing Claude API key in config");
        return false;
    }
    if (config.aiProvider === "openai" && !config.openaiApiKey) {
        console.error("Missing OpenAI API key in config");
        return false;
    }
    
    return true;
}

async function processScreenData(config: Config, provider: AIProvider) {
    const now = new Date();
    const oneDayAgo = new Date(now.getTime() - 24 * 60 * 60 * 1000);

    const screenData = await pipe.queryScreenpipe({
        start_time: oneDayAgo.toISOString(),
        end_time: now.toISOString(),
        limit: config.pageSize,
        content_type: config.contentType,
    });

    if (!screenData || !screenData.data || screenData.data.length === 0) {
        console.log("No screen data available for the past day");
        return;
    }

    try {
        console.log("Generating narrative summary");
        const narrativeSummary = await generateNarrativeSummary(screenData.data, provider, config);
        console.log("Generated narrative summary:", JSON.stringify(narrativeSummary, null, 2));
        await saveNarrativeSummary(narrativeSummary);
        
        const gistUrl = await createGist(narrativeSummary, config);
        if (gistUrl) {
            console.log("Created gist for review:", gistUrl);
        } else {
            console.log("Failed to create gist");
            console.error("Please check your GitHub personal access token and ensure it has the necessary permissions.");
        }
    } catch (summaryError) {
        console.error("Error generating or processing narrative summary:", summaryError);
        logError(summaryError);
    }
}

function logError(error: unknown) {
    if (error instanceof Error) {
        console.error("Error message:", error.message);
        console.error("Error stack:", error.stack);
    } else {
        console.error("Non-Error object thrown:", error);
    }
}

main().catch(error => {
    console.error("Fatal error in Screen Time Storyteller:", error);
    logError(error);
});