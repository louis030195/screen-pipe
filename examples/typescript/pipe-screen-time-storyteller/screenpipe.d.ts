declare global {
  const pipe: {
    /**
     * Reads the contents of a file.
     * @param path - The path to the file to be read.
     * @returns A promise that resolves with the file contents as a string.
     */
    readFile: (path: string) => Promise<string>;

    /**
     * Writes content to a file.
     * @param path - The path to the file to be written.
     * @param contents - The content to write to the file.
     * @returns A promise that resolves when the write operation is complete.
     */
    writeFile: (path: string, contents: string) => Promise<void>;

    /**
     * Removes a file.
     * @param path - The path to the file to be removed.
     * @returns A promise that resolves when the file is removed.
     */
    removeFile: (path: string) => Promise<void>;

    /**
     * Performs a GET request to the specified URL.
     * @param url - The URL to send the GET request to.
     * @returns A promise that resolves with the response data.
     */
    get: (url: string) => Promise<any>;

    /**
     * Performs a POST request to the specified URL.
     * @param url - The URL to send the POST request to.
     * @param body - The body of the POST request.
     * @returns A promise that resolves with the response data.
     */
    post: (url: string, body: string) => Promise<any>;

    /**
     * Performs a fetch request.
     * @param url - The URL to fetch.
     * @param options - Optional fetch options.
     * @returns A promise that resolves with the fetch response.
     */
    fetch: (url: string, options?: RequestInit) => Promise<Response>;

    /**
     * Sends a notification.
     * @param options - The notification options.
     * @returns A promise that resolves when the notification is sent.
     */
    sendNotification: (options: {
      title: string;
      body: string;
    }) => Promise<any>;

    /**
     * Loads the configuration for the pipe.
     * @returns A promise that resolves with the configuration object.
     */
    loadConfig: () => Promise<Record<string, any>>;

    /**
     * The current configuration of the pipe.
     */
    config: Record<string, any>;

    /**
     * Metadata about the current pipe.
     */
    metadata: {
      id: string;
    };

    /**
     * Sends an email.
     * @param options - The email options.
     * @returns A promise that resolves to true if the email was sent successfully, false otherwise.
     */
    sendEmail: (options: {
      to: string;
      from: string;
      password: string;
      subject: string;
      body: string;
    }) => Promise<boolean>;

    /**
     * Queries the Screenpipe database.
     * @param params - The query parameters.
     * @returns A promise that resolves with the query results or null if an error occurred.
     */
    queryScreenpipe: (
      params: ScreenpipeQueryParams
    ) => Promise<ScreenpipeResponse | null>;
  };

  const fs: {
    /**
     * Reads the contents of a file.
     * @param path - The path to the file to be read.
     * @returns A promise that resolves with the file contents as a string.
     */
    readFile: (path: string) => Promise<string>;

    /**
     * Writes content to a file.
     * @param path - The path to the file to be written.
     * @param contents - The content to write to the file.
     * @returns A promise that resolves when the write operation is complete.
     */
    writeFile: (path: string, contents: string) => Promise<void>;

    /**
     * Reads the contents of a directory.
     * @param path - The path to the directory to be read.
     * @returns A promise that resolves with an array of file names in the directory.
     */
    readdir: (path: string) => Promise<string[]>;

    /**
     * Creates a new directory.
     * @param path - The path of the directory to be created.
     * @returns A promise that resolves when the directory is created.
     */
    mkdir: (path: string) => Promise<void>;
  };

  const path: {
    /**
     * Joins multiple path segments into a single path.
     * @param paths - The path segments to join.
     * @returns The joined path string.
     */
    join: (...paths: string[]) => string;
  };

  namespace NodeJS {
    interface ProcessEnv {
      SCREENPIPE_DIR: string;
      SCREENPIPE_LOG_API_URL?: string;
      SCREENPIPE_SERVER_URL?: string;
      PIPE_DIR: string;
      NOTION_DATABASE_ID: string;
      NOTION_API_KEY: string;
    }
  }
  const process: {
    env: NodeJS.ProcessEnv;
  };

  /**
   * Types of content that can be queried in Screenpipe.
   */
  type ContentType = "ocr" | "audio" | "all";

  /**
   * Parameters for querying Screenpipe.
   */
  interface ScreenpipeQueryParams {
    q?: string;
    content_type?: ContentType;
    limit?: number;
    offset?: number;
    start_time?: string;
    end_time?: string;
    app_name?: string;
    window_name?: string;
    include_frames?: boolean;
    min_length?: number;
    max_length?: number;
  }

  /**
   * Structure of OCR (Optical Character Recognition) content.
   */
  interface OCRContent {
    frame_id: number;
    text: string;
    timestamp: string;
    file_path: string;
    offset_index: number;
    app_name: string;
    window_name: string;
    tags: string[];
    frame?: string;
  }

  /**
   * Structure of audio content.
   */
  interface AudioContent {
    chunk_id: number;
    transcription: string;
    timestamp: string;
    file_path: string;
    offset_index: number;
    tags: string[];
    device_name: string;
    device_type: string;
  }

  /**
   * Structure of Full Text Search content.
   */
  interface FTSContent {
    text_id: number;
    matched_text: string;
    frame_id: number;
    timestamp: string;
    app_name: string;
    window_name: string;
    file_path: string;
    original_frame_text?: string;
    tags: string[];
  }

  /**
   * Union type for different types of content items.
   */
  type ContentItem =
    | { type: "OCR"; content: OCRContent }
    | { type: "Audio"; content: AudioContent }
    | { type: "FTS"; content: FTSContent };

  /**
   * Pagination information for search results.
   */
  interface PaginationInfo {
    limit: number;
    offset: number;
    total: number;
  }

  /**
   * Structure of the response from a Screenpipe query.
   */
  interface ScreenpipeResponse {
    data: ContentItem[];
    pagination: PaginationInfo;
  }
}

export {};
